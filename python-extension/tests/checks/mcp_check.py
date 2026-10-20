"""mcp and what it stands on: jsonschema (every draft's metaschema, errors and their best match,
formats, $ref and $dynamicRef through a referencing Registry — rpds underneath), pyjwt (HMAC and
EdDSA tokens, through cryptography; expiry and audience errors), python-multipart (a form parsed
as it streams in), and mcp itself — an MCPServer's tools (structured output, errors, a tool that
reports progress and logs), resources and templates, prompts and completions — used by a Client
in-process (the modern protocol and the legacy initialize handshake), over stdio (this file run
again as the server, a process of its own), over streamable HTTP and over SSE (the server's Starlette
apps on uvicorn, on the loopback). Nothing printed depends on the time, a random id or the platform.
tests/reference.py runs it with the packages' own wheels.
"""
import asyncio
import json
import logging
import os
import socket
import sys
import threading
import warnings

from mcp.server import MCPServer
from mcp.server.mcpserver import Context
from mcp.shared.exceptions import MCPDeprecationWarning

# the server's logs of the failing tools, uvicorn's and httpx2's: on stderr, and not what is compared;
# the logging capability (ctx.info) is deprecated in the 2026-07-28 protocol, and still works
logging.disable(logging.CRITICAL)
warnings.simplefilter("ignore", MCPDeprecationWarning)


def make_server():
    server = MCPServer("check", instructions="a server for the check")

    @server.tool()
    def add(a: int, b: int) -> int:
        """Add two numbers."""
        return a + b

    @server.tool()
    def stats(values: list[float]) -> dict[str, float]:
        """Count, sum and mean."""
        return {"count": len(values), "sum": sum(values), "mean": sum(values) / len(values)}

    @server.tool()
    def fail(why: str) -> str:
        """Always fails."""
        raise ValueError(f"failed: {why}")

    @server.tool()
    async def steps(n: int, ctx: Context) -> str:
        """Reports its progress, and logs."""
        for i in range(n):
            await ctx.report_progress(i + 1, n, f"step {i + 1}")
        await ctx.info(f"{n} steps done")
        return f"{n} steps"

    @server.resource("config://app")
    def config() -> str:
        """The configuration."""
        return json.dumps({"debug": False, "level": 3})

    @server.resource("greeting://{name}")
    def greeting(name: str) -> str:
        """Greet someone by name."""
        return f"Hello, {name}!"

    @server.prompt()
    def review(code: str, language: str = "python") -> str:
        """Review some code."""
        return f"Review this {language} code:\n{code}"

    return server


async def session(client, label):
    print(f"[{label}] server", client.server_info.name, "instructions", client.instructions)
    tools = await client.list_tools()
    for t in sorted(tools.tools, key=lambda t: t.name):
        print(f"[{label}] tool", t.name, repr(t.description), json.dumps(t.input_schema, sort_keys=True))
    r = await client.call_tool("add", {"a": 2, "b": 40})
    print(f"[{label}] add", r.is_error, r.structured_content, [c.text for c in r.content])
    r = await client.call_tool("stats", {"values": [1.5, 2.5, 5]})
    print(f"[{label}] stats", r.is_error, json.dumps(r.structured_content, sort_keys=True))
    r = await client.call_tool("fail", {"why": "on purpose"})
    print(f"[{label}] fail", r.is_error, [c.text for c in r.content])
    r = await client.call_tool("add", {"a": "two", "b": 1})
    print(f"[{label}] add (invalid)", r.is_error, "validation error" in " ".join(c.text for c in r.content).lower())
    progress = []

    async def on_progress(p, total, message):
        progress.append((p, total, message))

    r = await client.call_tool("steps", {"n": 3}, progress_callback=on_progress)
    print(f"[{label}] steps", [c.text for c in r.content], progress)
    resources = await client.list_resources()
    print(f"[{label}] resources", sorted((str(x.uri), x.name, x.mime_type) for x in resources.resources))
    templates = await client.list_resource_templates()
    print(f"[{label}] templates", sorted((x.uri_template, x.name) for x in templates.resource_templates))
    for uri in ("config://app", "greeting://Ada"):
        r = await client.read_resource(uri)
        print(f"[{label}] read", uri, [(str(c.uri), c.text) for c in r.contents])
    prompts = await client.list_prompts()
    print(f"[{label}] prompts", [(p.name, [(a.name, a.required) for a in p.arguments]) for p in prompts.prompts])
    r = await client.get_prompt("review", {"code": "print(1)"})
    print(f"[{label}] prompt", [(m.role, m.content.text) for m in r.messages])


async def in_process():
    from mcp import Client
    for mode in ("auto", "legacy"):
        async with Client(make_server(), mode=mode) as client:
            await session(client, f"in-process {mode}")


async def over_stdio():
    from mcp import Client, StdioServerParameters
    params = StdioServerParameters(command=sys.executable, args=[os.path.abspath(__file__), "serve-stdio"],
                                   env=dict(os.environ))
    logs = []

    async def on_log(params):
        logs.append((params.level, params.data))

    async with Client(params, mode="legacy", logging_callback=on_log) as client:
        await session(client, "stdio")
    print("[stdio] logs", logs)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


async def over_http(kind):
    """The server's Starlette app on uvicorn, on the loopback: streamable HTTP, or SSE (sse-starlette)."""
    import uvicorn
    from mcp import Client
    from mcp.client.sse import sse_client
    port = free_port()
    server = make_server()
    app = server.streamable_http_app() if kind == "http" else server.sse_app()
    web = uvicorn.Server(uvicorn.Config(app, host="127.0.0.1", port=port, log_level="error", lifespan="on"))
    thread = threading.Thread(target=web.run, daemon=True)
    thread.start()
    for _ in range(500):
        if web.started:
            break
        await asyncio.sleep(0.02)
    print(f"[{kind}] uvicorn started", web.started)
    try:
        if kind == "http":
            client = Client(f"http://127.0.0.1:{port}/mcp")
        else:
            client = Client(sse_client(f"http://127.0.0.1:{port}/sse"), mode="legacy")
        async with client:
            await session(client, kind)
    finally:
        web.should_exit = True
        thread.join(30)
    print(f"[{kind}] uvicorn stopped", not thread.is_alive())


def check_jsonschema():
    import jsonschema
    import jsonschema_specifications
    import referencing
    from jsonschema import Draft202012Validator, FormatChecker, validators
    from jsonschema.exceptions import best_match
    from referencing import Registry, Resource
    from referencing.jsonschema import DRAFT202012

    for v in (jsonschema.Draft3Validator, jsonschema.Draft4Validator, jsonschema.Draft6Validator, jsonschema.Draft7Validator,
              jsonschema.Draft201909Validator, jsonschema.Draft202012Validator):
        v.check_schema(v.META_SCHEMA)
        print("metaschema", v.__name__, v.META_SCHEMA["$schema"], validators.validator_for(v.META_SCHEMA) is v)
    print("specifications", len(list(jsonschema_specifications.REGISTRY)))
    schema = {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "name": {"type": "string", "minLength": 2},
            "age": {"type": "integer", "minimum": 0, "maximum": 150},
            "email": {"type": "string", "format": "email"},
            "ip": {"type": "string", "format": "ipv4"},
            "when": {"type": "string", "format": "date"},
            "tags": {"type": "array", "items": {"enum": ["a", "b", "c"]}, "uniqueItems": True},
            "point": {"$ref": "urn:check:point"},
            "tree": {"$ref": "#/$defs/node"},
        },
        "required": ["name", "age"],
        "unevaluatedProperties": False,
        "$defs": {"node": {"type": "object", "properties": {"value": {"type": "number"},
                                                              "children": {"type": "array", "items": {"$ref": "#/$defs/node"}}},
                           "required": ["value"]}},
    }
    point = Resource.from_contents({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "urn:check:point",
                                    "type": "object", "properties": {"x": {"type": "number"}, "y": {"type": "number"}},
                                    "required": ["x", "y"], "additionalProperties": False})
    registry = Registry().with_resources([("urn:check:point", point)])
    v = Draft202012Validator(schema, registry=registry, format_checker=FormatChecker())
    good = {"name": "Ada", "age": 36, "email": "ada@example.org", "ip": "10.0.0.1", "when": "1843-07-10",
            "tags": ["a", "b"], "point": {"x": 1, "y": 2.5}, "tree": {"value": 1, "children": [{"value": 2, "children": []}]}}
    print("valid", v.is_valid(good), list(v.iter_errors(good)))
    bad = {"name": "A", "age": -1, "email": "no-at", "ip": "300.1.1.1", "when": "1843-13-40", "tags": ["a", "a", "z"],
           "point": {"x": 1, "z": 3}, "tree": {"children": [{"value": "x"}]}, "extra": True}
    for e in sorted(v.iter_errors(bad), key=lambda e: (list(e.absolute_path), e.validator, e.message)):
        print("error", list(e.absolute_path), e.validator, e.message)
    print("best match", best_match(v.iter_errors(bad)).message)
    try:
        jsonschema.validate({"name": 5, "age": 1}, schema, registry=registry)
    except jsonschema.ValidationError as e:
        print("validate", type(e).__name__, e.message, list(e.schema_path))
    try:
        Draft202012Validator.check_schema({"type": "nothing"})
    except jsonschema.SchemaError as e:
        print("schema error", e.message[:60])
    # $dynamicRef: a list whose items an extending schema narrows
    generic = Resource.from_contents({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "urn:check:list",
                                      "$dynamicAnchor": "item", "type": "array", "items": {"$dynamicRef": "#item"},
                                      "$defs": {"item": {"$dynamicAnchor": "item"}}})
    strings = {"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "urn:check:strings",
               "$ref": "urn:check:list", "$defs": {"item": {"$dynamicAnchor": "item", "type": "string"}}}
    registry2 = registry.with_resource("urn:check:list", generic)
    dv = Draft202012Validator(strings, registry=registry2)
    print("dynamicRef", dv.is_valid(["a", "b"]), dv.is_valid(["a", 1]))
    # the registry: lookups, anchors, a crawl
    resolver = registry2.resolver()
    print("lookup", resolver.lookup("urn:check:point#/properties/x").contents, sorted(registry2.crawl()))
    anchored = DRAFT202012.create_resource({"$id": "urn:check:a", "$defs": {"n": {"$anchor": "num", "type": "number"}}})
    r3 = (anchored @ Registry()).crawl()
    print("anchor", r3.resolver().lookup("urn:check:a#num").contents, referencing.__name__)


def check_jwt():
    import jwt
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric import ed25519
    payload = {"sub": "ada", "aud": "check", "iat": 1700000000, "exp": 4102444800, "scope": ["read", "write"]}
    token = jwt.encode(payload, "secret-key-of-thirty-two-bytes!!", algorithm="HS256", headers={"kid": "k1"})
    print("hs256", token)
    print("hs256 decoded", jwt.decode(token, "secret-key-of-thirty-two-bytes!!", algorithms=["HS256"], audience="check"))
    print("header", jwt.get_unverified_header(token))
    key = ed25519.Ed25519PrivateKey.from_private_bytes(bytes(range(32)))
    pem = key.public_key().public_bytes(serialization.Encoding.PEM, serialization.PublicFormat.SubjectPublicKeyInfo)
    token = jwt.encode(payload, key, algorithm="EdDSA")
    print("eddsa", token)
    print("eddsa decoded", jwt.decode(token, pem, algorithms=["EdDSA"], audience="check")["scope"])
    jwk = jwt.PyJWK.from_dict(json.loads(jwt.algorithms.OKPAlgorithm.to_jwk(key.public_key())))
    print("jwk", jwk.key_type, jwk.algorithm_name, jwt.decode(token, jwk, algorithms=["EdDSA"], audience="check")["sub"])
    for wrong, what in ((dict(payload, exp=1000000000), "expired"), (dict(payload, aud="other"), "audience")):
        try:
            jwt.decode(jwt.encode(wrong, "k" * 32, algorithm="HS256"), "k" * 32, algorithms=["HS256"], audience="check")
        except jwt.PyJWTError as e:
            print("jwt", what, type(e).__name__, e)
    try:
        jwt.decode(token, "k" * 32, algorithms=["HS256"])
    except jwt.PyJWTError as e:
        print("jwt wrong algorithm", type(e).__name__)


def check_multipart():
    from multipart.multipart import parse_options_header
    from python_multipart import MultipartParser
    boundary = b"----check"
    body = (b"------check\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nHello\r\n"
            b"------check\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\n"
            b"Content-Type: text/plain\r\n\r\nline one\r\nline two\r\n------check--\r\n")
    parts, current = [], {}

    def on_header_field(data, start, end):
        current.setdefault("field", b"")
        current["field"] += data[start:end]

    def on_header_value(data, start, end):
        current.setdefault("value", b"")
        current["value"] += data[start:end]

    def on_header_end():
        current.setdefault("headers", []).append((current.pop("field"), current.pop("value")))

    def on_part_data(data, start, end):
        current.setdefault("data", b"")
        current["data"] += data[start:end]

    def on_part_end():
        parts.append((current.get("headers"), current.get("data")))
        current.clear()

    p = MultipartParser(boundary, {"on_header_field": on_header_field, "on_header_value": on_header_value,
                                   "on_header_end": on_header_end, "on_part_data": on_part_data, "on_part_end": on_part_end})
    for i in range(0, len(body), 7):   # as it streams in
        p.write(body[i:i + 7])
    p.finalize()
    for headers, data in parts:
        print("part", headers, data)
    print("options", parse_options_header(b'form-data; name="file"; filename="a.txt"'))


def main():
    import attrs
    import click
    import jsonschema
    import jwt
    import mcp
    import mcp_types
    import opentelemetry.version
    import python_multipart
    import referencing
    import rpds
    import sse_starlette
    import starlette
    import uvicorn
    from importlib.metadata import version
    print("versions", version("mcp"), version("mcp-types"), mcp_types.LATEST_PROTOCOL_VERSION, jsonschema.__name__,
          version("jsonschema"), version("referencing"), version("rpds-py"), attrs.__version__, jwt.__version__,
          python_multipart.__version__, starlette.__version__, uvicorn.__version__, version("sse-starlette"),
          click.__name__, opentelemetry.version.__version__, mcp.__name__, referencing.__name__, sse_starlette.__name__,
          rpds.__name__)
    check_jsonschema()
    check_jwt()
    check_multipart()
    asyncio.run(in_process())
    asyncio.run(over_stdio())
    asyncio.run(over_http("http"))
    asyncio.run(over_http("sse"))
    print("done")


if __name__ == "__main__":
    if sys.argv[1:] == ["serve-stdio"]:
        make_server().run("stdio")
    else:
        main()
