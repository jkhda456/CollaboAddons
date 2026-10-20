"""cryptography and the OpenSSL in it: every hash, MAC and KDF; AES in every mode, the AEADs,
ChaCha20, Camellia, SM4 and the legacy ciphers (the legacy provider, built in); padding and key
wrapping; RSA (PKCS#1 v1.5 signatures, PSS, OAEP), DSA, ECDSA (deterministic, RFC 6979) and ECDH,
Ed25519, Ed448, X25519, X448, finite-field DH; keys in PEM, DER and OpenSSH, encrypted and not; SSH
certificates; X.509 certificates, CSRs and CRLs built, signed, parsed and a chain verified; PKCS#7
and PKCS#12; Fernet, HOTP/TOTP; the errors it raises; threads. Keys and inputs are fixed (the keys
below were made once with cryptography itself), so what is printed is what the PyPI wheel prints;
what is random (OAEP, PSS, ECDSA without RFC 6979, encryption of keys) is printed as round trips.
tests/reference.py runs it with cryptography's own wheel.
"""
import datetime
import hashlib
import threading
import warnings

import cryptography
from cryptography import exceptions, x509
from cryptography.fernet import Fernet, InvalidToken, MultiFernet
from cryptography.hazmat.bindings._rust import openssl as rust_openssl
from cryptography.hazmat.decrepit.ciphers import algorithms as decrepit
from cryptography.hazmat.primitives import cmac, constant_time, hashes, hmac, keywrap, padding, poly1305, serialization
from cryptography.hazmat.primitives.asymmetric import dh, dsa, ec, ed448, ed25519, rsa, x448, x25519
from cryptography.hazmat.primitives.asymmetric import padding as apad
from cryptography.hazmat.primitives.asymmetric import utils as autils
from cryptography.hazmat.primitives.ciphers import Cipher, aead, algorithms, modes
from cryptography.hazmat.primitives.kdf import concatkdf, hkdf, kbkdf, pbkdf2, scrypt, x963kdf
from cryptography.hazmat.primitives.serialization import pkcs7, pkcs12
from cryptography.hazmat.primitives.twofactor import hotp, totp
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID

warnings.simplefilter("ignore", cryptography.utils.CryptographyDeprecationWarning)   # (finite-field DH)

RSA_PEM = b"""-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDJlZep8gwJD1FD
v755K3BsRv9GNEYySosBtZPjrKG1qLzzxghD+JHhvOf9VcrzllwJLcshQ7kMdB+s
DqP/0xW8zQ53ythQg+dZIe7tj9CBHRG+3kXhV+Q1ANVQZWCu/4qq2wLvA6BkxZXE
PilLM80Utkhgrq/1QzkG2ks8j8DTjrEaLTme94HU/6t8TalP6QVmuG/ALrSuMNcm
GwEWfa4J2vlAoUlBfYdxynRc+wuctDZjsCnDDd0U/49JacWq4iGspagRKzW+vCVo
LdiLK2r9uTCNiwkfW8xuuu0JXrmJpjVAQo5YVlBD9+JNAWK1nl4V3unjjGlSHWfb
cDCzn5SxAgMBAAECggEAGwfGmaCzEXHu3S1Jvq4JpKiT1AJFbrqaeiUIICLcCjkG
oToq4zu3tV54eHq3vJhVs1tjHgzv3tDMFcJe0Pp+iIhcih6hM+aur7/XmHOiK0tO
alrGtwlAjgTpwEk1FjIo53EiVbzAQKhSmp5qjhgeRfU4A6guQFkpEpdUja80VYmo
iZYxHvkaWzurdFVXlisFpa/q6TG2HbpjzRYdLDRJFhdM3helDkzdVlAg4EvTpAb8
DNCTUBUckX0uSgGqeopD6KgTARA0yLIEEO2AEc2UMOkXzeldmaT49fWWA1/yG2eZ
40qhFsDV+f0N/chHuT0aLmSBeLTJ1cwsdml1BUWiSwKBgQDvdLHwTfBTgrD777uD
gGngueitP9A0WlEO2q5hM+ri9cowS4z45Nzno+VchPm/V46b1qE1I3kK/7qFQGDW
b9OShVBI3QDBh+KBJRLy+wG1LiETRam1jSvK5JvcPNBooSI0Szbhu25VKjxfJ8Ov
ZuhecyHjFmEXz4K/vXSL8y6HZwKBgQDXgw5w/kUAleGZOsu+DMBjT4qUfsSqPouL
O0OKwrIF7U4Sx2V9DjalVYLi62GQUyaGIYvO39yKxZktMPDqkE6Zye8+NC8ZmEkF
oAMe89N7aHYmnz6YLfbkKagsHtGYVSPHntYSTvaI74e6Ms2qo/B1HfeyJRgYcI6e
MgmLw9rsJwKBgQCJMdYldbJFVrLqlg9cp72B+WJbqR00ca7RkO+qV6GzidnfU9og
XJhIy7nFCZdCTvsxRIa2uG1IpP6MDDevhQC2iVVXefBN8foPvlseBjyTPyvKK/ax
Q4IczGCUje9mkzPSvGlkagwsFXfjmRfgLljijJqUzqSwjL8/9l+JM7qr7wKBgQCQ
QxyBqtPJ5orTpbYJDEQktnQPnyRKyf9jPlKiKl489XNTwsI/CiquViV9fQ8cK20Y
UJBj9RqOkmNNCvvaKKtU6wYJX6k/ut72svwj1hf0/iJJDEQ8Xlhsty3qCt9G14dR
8xNdwhnp5F242JUv6vnljB/o0kfk5EOzxc76DisX9QKBgGXnLiy0l0e9pm6Phteg
uhl3M5nKt+rwtM7i6PGPM+Za9LCmRtApCIXbOFpWnbVehAcALVBYtG8/GQ4LxsZR
0zPdvKMJ8OdVkmmCBfWAeehvVcuHBA/HsJ3a+QLQ/brSaZRA6Dk7dWn0b4wpaPGC
mIFbaOPamcb2laKooTDUXceg
-----END PRIVATE KEY-----"""
DSA_PEM = b"""-----BEGIN PRIVATE KEY-----
MIICZAIBADCCAjkGByqGSM44BAEwggIsAoIBAQCMl+m+c411HSkl6NprNwNEgZyQ
/WUIBsqknCk6G/MWAxYhFUlapYD7ujYl+y5k+Pnt9abzTTD5HJnzUqnnFbW/dty9
iyqI9U5fr9Z0ub3MYDqmfCwOi9gcylKYdv2GkX2F6n0h4fEpmaZRyEWBH48luZ41
Wa+SdSwg59Yk/3UdnqiLStuMPcmHI7G+79jZ3JdQpnSTkVZNFzk6JVG79QP34Pgm
aireCI+MWVqI5iq/ObarU3GsMzWiMrY16e3V0QlLdlDPa5/4Bp87Dtku1is5as+W
n3os5It1j8xkethMs5krRhS4zQMnn68pGUH9Tkm73NRH705P/i1kHPxt4DftAiEA
8I1xuPQcAzPUI0Im54hc/IgC3kSk4wrq+5gOdMuyEGcCggEAfCkzjjp5/rouDwd4
nJrMV8pV9AKc0HAy7NfwaZkW1o06csTY9hNrcIAHIzVIaYlfbHHD6dID0099O3RC
mrNz6TvPb/oFrGiDhUOCiEvgJuFGB2A00aA3u+smeZyfvyCGBwlVqSJWXWUfpwjg
Jf9AxRZIqLMLMv62r4gRH6DoNd56RjOAviENr3lhD9VdncrqRqnpzx5l8igcpKS6
4Q7ia7ks8w89gXl6oJ/9xDQcAbQLrebVjAfmmQ5broB7Mmnza008HLBYqRA2AqNU
Db32JPSfva6w1702eANOuz5bw8F+lWq1IvGF2cGDttEVFLZhFcuiIxUuYNR79Bcx
KpIkdQQiAiATEDbHDnkQ3CbE10rVTMnwzIJbNFU2a2YqYt8yBcvkeg==
-----END PRIVATE KEY-----"""
EC_PEM = b"""-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg/mosdKnesAmnzvR5
PfhLSgQ9253r5X0hvwXYz2t/SmOhRANCAARgmI9V3zK0t9dUopzTDP+51GGLEEnJ
TF293Cm5/FRq3YVxVVjPQAlBVNj6ZiOkVZayInA8xEmDnX5CLrHhcFd0
-----END PRIVATE KEY-----"""


def h(b):
    return hashlib.sha256(b).hexdigest()[:32]


print("versions", cryptography.__version__, rust_openssl.openssl_version_text(), hex(rust_openssl.openssl_version()))
print("providers", "legacy", rust_openssl._legacy_provider_loaded, "fips", rust_openssl.is_fips_enabled())

# ── hashes, MACs, KDFs ──────────────────────────────────────────────────────────────────────────
DATA = b"The quick brown fox jumps over the lazy dog" * 37
for algo in [hashes.MD5(), hashes.SHA1(), hashes.SHA224(), hashes.SHA256(), hashes.SHA384(), hashes.SHA512(),
             hashes.SHA512_224(), hashes.SHA512_256(), hashes.SHA3_224(), hashes.SHA3_256(), hashes.SHA3_384(),
             hashes.SHA3_512(), hashes.BLAKE2b(64), hashes.BLAKE2s(32), hashes.SM3(), hashes.SHAKE128(32),
             hashes.SHAKE256(64)]:
    d = hashes.Hash(algo)
    d.update(DATA[:100])
    c = d.copy()
    d.update(DATA[100:])
    print("hash", algo.name, d.finalize().hex(), c.finalize().hex()[:16])
for algo in [hashes.SHA256(), hashes.SHA3_256(), hashes.SHA512(), hashes.BLAKE2b(64)]:
    m = hmac.HMAC(b"key" * 11, algo)
    m.update(DATA)
    print("hmac", algo.name, m.finalize().hex())
c = cmac.CMAC(algorithms.AES(bytes(range(16))))
c.update(DATA)
print("cmac", c.finalize().hex())
print("poly1305", poly1305.Poly1305.generate_tag(bytes(range(32)), DATA).hex())
salt = b"NaCl-salt-16byte"
print("hkdf", hkdf.HKDF(hashes.SHA256(), 42, salt, b"info").derive(b"input key").hex())
print("hkdf-expand", hkdf.HKDFExpand(hashes.SHA512(), 32, b"ctx").derive(bytes(64)).hex())
print("pbkdf2", pbkdf2.PBKDF2HMAC(hashes.SHA256(), 32, salt, 1000).derive(b"password").hex())
print("scrypt", scrypt.Scrypt(salt, 32, 2**10, 8, 1).derive(b"password").hex())
print("concatkdf", concatkdf.ConcatKDFHash(hashes.SHA256(), 32, b"other").derive(b"shared").hex())
print("x963kdf", x963kdf.X963KDF(hashes.SHA256(), 32, b"shared-info").derive(b"secret").hex())
print("kbkdf", kbkdf.KBKDFHMAC(hashes.SHA256(), kbkdf.Mode.CounterMode, 32, 4, 4, kbkdf.CounterLocation.BeforeFixed,
                                b"label", b"context", None).derive(b"key material").hex())
try:
    from cryptography.hazmat.primitives.kdf.argon2 import Argon2id
    print("argon2id", Argon2id(salt=salt, length=32, iterations=2, lanes=2, memory_cost=1024).derive(b"pw").hex())
except exceptions.UnsupportedAlgorithm as e:
    print("argon2id unsupported", e)

# ── symmetric ciphers ───────────────────────────────────────────────────────────────────────────
K16, K32, IV = bytes(range(16)), bytes(range(32)), bytes(range(100, 116))
PT = bytes(range(256)) * 2


def run(cipher, data=PT):
    e = cipher.encryptor()
    ct = e.update(data) + e.finalize()
    d = cipher.decryptor()
    return ct, d.update(ct) + d.finalize() == data


for name, mode in [("cbc", modes.CBC(IV)), ("ctr", modes.CTR(IV)), ("ofb", modes.OFB(IV)), ("cfb", modes.CFB(IV)),
                   ("cfb8", modes.CFB8(IV)), ("ecb", modes.ECB())]:
    for key in (K16, K32):
        ct, ok = run(Cipher(algorithms.AES(key), mode))
        print("aes", len(key) * 8, name, h(ct), ok)
ct, ok = run(Cipher(algorithms.AES(K32 + K32[::-1]), modes.XTS(IV)))
print("aes xts", h(ct), ok)
e = Cipher(algorithms.AES(K16), modes.GCM(IV[:12])).encryptor()
e.authenticate_additional_data(b"aad")
ct = e.update(PT) + e.finalize()
print("aes gcm", h(ct), e.tag.hex())
d = Cipher(algorithms.AES(K16), modes.GCM(IV[:12], e.tag)).decryptor()
d.authenticate_additional_data(b"aad")
print("aes gcm decrypt", d.update(ct) + d.finalize() == PT)
for name, a, nonce in [("AESGCM", aead.AESGCM(K32), IV[:12]), ("AESCCM", aead.AESCCM(K16), IV[:13]),
                       ("AESOCB3", aead.AESOCB3(K16), IV[:12]), ("AESGCMSIV", aead.AESGCMSIV(K32), IV[:12]),
                       ("ChaCha20Poly1305", aead.ChaCha20Poly1305(K32), IV[:12])]:
    ct = a.encrypt(nonce, PT, b"aad")
    print("aead", name, h(ct), ct[-16:].hex(), a.decrypt(nonce, ct, b"aad") == PT)
    try:
        a.decrypt(nonce, ct[:-1] + bytes([ct[-1] ^ 1]), b"aad")
    except exceptions.InvalidTag:
        print("aead", name, "tampered: InvalidTag")
siv = aead.AESSIV(K32)
ct = siv.encrypt(PT, [b"header", IV])
print("aead AESSIV", h(ct), siv.decrypt(ct, [b"header", IV]) == PT)
ct, ok = run(Cipher(algorithms.ChaCha20(K32, IV), mode=None))
print("chacha20", h(ct), ok)
for name, algo, mode in [("camellia", algorithms.Camellia(K16), modes.CBC(IV)), ("sm4", algorithms.SM4(K16), modes.CBC(IV)),
                         ("3des", decrepit.TripleDES(bytes(range(24))), modes.CBC(IV[:8])),
                         ("blowfish", decrepit.Blowfish(K16), modes.CBC(IV[:8])),
                         ("cast5", decrepit.CAST5(K16), modes.CBC(IV[:8])), ("seed", decrepit.SEED(K16), modes.CBC(IV)),
                         ("idea", decrepit.IDEA(K16), modes.CBC(IV[:8])), ("rc4", decrepit.ARC4(K16), None)]:
    try:
        ct, ok = run(Cipher(algo, mode))
        print("cipher", name, h(ct), ok)
    except exceptions.UnsupportedAlgorithm as e:
        print("cipher", name, "unsupported", e)
for name, pad in [("pkcs7", padding.PKCS7(128)), ("ansix923", padding.ANSIX923(64))]:
    p = pad.padder()
    padded = p.update(b"odd-length data") + p.finalize()
    u = pad.unpadder()
    print("padding", name, padded.hex(), u.update(padded) + u.finalize())
wrapped = keywrap.aes_key_wrap(K16, K32)
print("keywrap", wrapped.hex(), keywrap.aes_key_unwrap(K16, wrapped) == K32)
wrapped = keywrap.aes_key_wrap_with_padding(K16, b"twenty bytes of key!")
print("keywrap padded", wrapped.hex(), keywrap.aes_key_unwrap_with_padding(K16, wrapped))

# ── RSA, DSA, EC ────────────────────────────────────────────────────────────────────────────────
rk = serialization.load_pem_private_key(RSA_PEM, None)
pn = rk.public_key().public_numbers()
print("rsa key", rk.key_size, pn.e, hex(pn.n)[:24])
sig = rk.sign(DATA, apad.PKCS1v15(), hashes.SHA256())
print("rsa pkcs1v15", sig.hex()[:64], rk.public_key().verify(sig, DATA, apad.PKCS1v15(), hashes.SHA256()))
digest = hashlib.sha384(DATA).digest()
sig2 = rk.sign(digest, apad.PKCS1v15(), autils.Prehashed(hashes.SHA384()))
print("rsa prehashed", h(sig2), rk.public_key().recover_data_from_signature(sig2, apad.PKCS1v15(), hashes.SHA384()) == digest)
pss = apad.PSS(apad.MGF1(hashes.SHA256()), apad.PSS.MAX_LENGTH)
sig = rk.sign(DATA, pss, hashes.SHA256())
print("rsa pss", len(sig), rk.public_key().verify(sig, DATA, pss, hashes.SHA256()))
oaep = apad.OAEP(apad.MGF1(hashes.SHA256()), hashes.SHA256(), b"label")
print("rsa oaep", rk.decrypt(rk.public_key().encrypt(b"secret message", oaep), oaep))
try:
    rk.public_key().verify(sig, DATA + b"x", pss, hashes.SHA256())
except exceptions.InvalidSignature:
    print("rsa tampered: InvalidSignature")
priv = rk.private_numbers()
print("rsa recover", sorted(rsa.rsa_recover_prime_factors(pn.n, pn.e, priv.d)) == sorted([priv.p, priv.q]),
      rsa.rsa_crt_iqmp(priv.p, priv.q) == priv.iqmp)
dk = serialization.load_pem_private_key(DSA_PEM, None)
sig = dk.sign(DATA, hashes.SHA256())
print("dsa", dk.key_size, dk.public_key().verify(sig, DATA, hashes.SHA256()))
ek = serialization.load_pem_private_key(EC_PEM, None)
sig = ek.sign(DATA, ec.ECDSA(hashes.SHA256(), deterministic_signing=True))
print("ecdsa rfc6979", sig.hex(), ek.public_key().verify(sig, DATA, ec.ECDSA(hashes.SHA256())))
print("ecdsa decode", [hex(v)[:18] for v in autils.decode_dss_signature(sig)])
sig = ek.sign(DATA, ec.ECDSA(hashes.SHA256()))
print("ecdsa random", ek.public_key().verify(sig, DATA, ec.ECDSA(hashes.SHA256())))
for curve in [ec.SECP256R1(), ec.SECP384R1(), ec.SECP521R1(), ec.SECP256K1(), ec.BrainpoolP256R1()]:
    a = ec.derive_private_key(123456789, curve)
    b = ec.derive_private_key(987654321, curve)
    shared = a.exchange(ec.ECDH(), b.public_key())
    pub = a.public_key().public_bytes(serialization.Encoding.X962, serialization.PublicFormat.CompressedPoint)
    print("ecdh", curve.name, shared.hex()[:32], pub.hex()[:24], shared == b.exchange(ec.ECDH(), a.public_key()))
print("ec point", ec.EllipticCurvePublicKey.from_encoded_point(ec.SECP256R1(), ek.public_key().public_bytes(
    serialization.Encoding.X962, serialization.PublicFormat.UncompressedPoint)).public_numbers().x == ek.public_key().public_numbers().x)

# ── Ed25519, Ed448, X25519, X448, DH ────────────────────────────────────────────────────────────
e1 = ed25519.Ed25519PrivateKey.from_private_bytes(bytes(range(32)))
sig = e1.sign(DATA)
print("ed25519", e1.public_key().public_bytes_raw().hex(), sig.hex()[:64], e1.public_key().verify(sig, DATA))
e4 = ed448.Ed448PrivateKey.from_private_bytes(bytes(range(57)))
sig = e4.sign(DATA)
print("ed448", e4.public_key().public_bytes_raw().hex()[:32], sig.hex()[:64], e4.public_key().verify(sig, DATA))
xa, xb = x25519.X25519PrivateKey.from_private_bytes(bytes(32 * [7])), x25519.X25519PrivateKey.from_private_bytes(bytes(32 * [9]))
print("x25519", xa.exchange(xb.public_key()).hex(), xa.exchange(xb.public_key()) == xb.exchange(xa.public_key()))
ya, yb = x448.X448PrivateKey.from_private_bytes(bytes(56 * [7])), x448.X448PrivateKey.from_private_bytes(bytes(56 * [9]))
print("x448", ya.exchange(yb.public_key()).hex()[:32])
P14 = int("FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DD"
          "EF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED"
          "EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F"
          "83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3B"
          "E39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6955817183995497CEA956AE515D2261898FA0510"
          "15728E5A8AACAA68FFFFFFFFFFFFFFFF", 16)
params = dh.DHParameterNumbers(P14, 2)
da = dh.DHPrivateNumbers(1234567890123456789, dh.DHPublicNumbers(pow(2, 1234567890123456789, P14), params)).private_key()
db = dh.DHPrivateNumbers(987654321987654321, dh.DHPublicNumbers(pow(2, 987654321987654321, P14), params)).private_key()
print("dh", da.exchange(db.public_key()).hex()[:32], da.exchange(db.public_key()) == db.exchange(da.public_key()))

# ── serialization ───────────────────────────────────────────────────────────────────────────────
E, PF, PUF = serialization.Encoding, serialization.PrivateFormat, serialization.PublicFormat
for name, key in [("rsa", rk), ("dsa", dk), ("ec", ek), ("ed25519", e1), ("ed448", e4), ("x25519", xa)]:
    der = key.private_bytes(E.DER, PF.PKCS8, serialization.NoEncryption())
    pub = key.public_key().public_bytes(E.PEM, PUF.SubjectPublicKeyInfo)
    enc = key.private_bytes(E.PEM, PF.PKCS8, serialization.BestAvailableEncryption(b"pass phrase"))
    back = serialization.load_pem_private_key(enc, b"pass phrase")
    print("serialize", name, h(der), h(pub), back.private_bytes(E.DER, PF.PKCS8, serialization.NoEncryption()) == der)
for name, key in [("rsa", rk), ("ec", ek), ("ed25519", e1)]:
    ssh = key.public_key().public_bytes(E.OpenSSH, PUF.OpenSSH)
    priv = key.private_bytes(E.PEM, PF.OpenSSH, serialization.NoEncryption())   # (encrypted: bcrypt's)
    back = serialization.load_ssh_private_key(priv, None)
    print("openssh", name, ssh[:40], serialization.load_ssh_public_key(ssh).public_bytes(E.OpenSSH, PUF.OpenSSH) == ssh,
          back.public_key().public_bytes(E.OpenSSH, PUF.OpenSSH) == ssh)
print("traditional rsa", h(rk.private_bytes(E.PEM, PF.TraditionalOpenSSL, serialization.NoEncryption())),
      h(rk.public_key().public_bytes(E.PEM, PUF.PKCS1)))
cert = (serialization.SSHCertificateBuilder().public_key(ek.public_key()).serial(42).type(serialization.SSHCertificateType.USER)
        .key_id(b"key-id").valid_principals([b"alice"]).valid_after(1700000000).valid_before(1800000000)
        .add_critical_option(b"force-command", b"echo hi").add_extension(b"permit-pty", b"").sign(e1))
blob = cert.public_bytes()
loaded = serialization.load_ssh_public_identity(blob)
print("ssh cert", loaded.serial, loaded.key_id, loaded.valid_principals, loaded.critical_options, loaded.extensions)
loaded.verify_cert_signature()
print("ssh cert verified", loaded.signature_key().public_bytes(E.OpenSSH, PUF.OpenSSH) == e1.public_key().public_bytes(E.OpenSSH, PUF.OpenSSH))

# ── X.509 ───────────────────────────────────────────────────────────────────────────────────────
T0 = datetime.datetime(2026, 1, 1, tzinfo=datetime.timezone.utc)
ca_key = rk   # (the Web PKI's policy has no Ed25519 CAs; PKCS#1 v1.5 signatures are deterministic)
leaf_key = ec.derive_private_key(424242, ec.SECP256R1())
ca_name = x509.Name([x509.NameAttribute(NameOID.COUNTRY_NAME, "KR"), x509.NameAttribute(NameOID.ORGANIZATION_NAME, "collaboCore"),
                     x509.NameAttribute(NameOID.COMMON_NAME, "Test CA")])
ca = (x509.CertificateBuilder().subject_name(ca_name).issuer_name(ca_name).public_key(ca_key.public_key())
      .serial_number(1001).not_valid_before(T0).not_valid_after(T0 + datetime.timedelta(days=3650))
      .add_extension(x509.BasicConstraints(ca=True, path_length=1), critical=True)
      .add_extension(x509.KeyUsage(False, False, False, False, False, True, True, False, False), critical=True)
      .add_extension(x509.SubjectKeyIdentifier.from_public_key(ca_key.public_key()), critical=False)
      .sign(ca_key, hashes.SHA256()))
print("ca", h(ca.public_bytes(E.DER)), ca.subject.rfc4514_string(), ca.serial_number, ca.signature_algorithm_oid._name)
leaf = (x509.CertificateBuilder().subject_name(x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "example.com")]))
        .issuer_name(ca_name).public_key(leaf_key.public_key()).serial_number(2002)
        .not_valid_before(T0).not_valid_after(T0 + datetime.timedelta(days=90))
        .add_extension(x509.SubjectAlternativeName([x509.DNSName("example.com"), x509.DNSName("*.example.com"),
                                                    x509.IPAddress(__import__("ipaddress").ip_address("192.0.2.1"))]), critical=False)
        .add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH]), critical=False)
        .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(ca_key.public_key()), critical=False)
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(x509.KeyUsage(True, False, False, False, False, False, False, False, False), critical=True)
        .sign(ca_key, hashes.SHA256()))
print("leaf", h(leaf.public_bytes(E.DER)), [str(n.value) for n in leaf.extensions.get_extension_for_class(x509.SubjectAlternativeName).value])
parsed = x509.load_pem_x509_certificate(leaf.public_bytes(E.PEM))
print("parsed", parsed == leaf, parsed.not_valid_after_utc, parsed.version, [e.oid._name for e in parsed.extensions],
      parsed.fingerprint(hashes.SHA256()).hex()[:32])
parsed.verify_directly_issued_by(ca)
print("issued by", True)
from cryptography.x509 import verification
store = verification.Store([ca])
verifier = verification.PolicyBuilder().store(store).time(T0 + datetime.timedelta(days=10)).build_server_verifier(x509.DNSName("www.example.com"))
chain = verifier.verify(leaf, [])
print("verified chain", [c.subject.rfc4514_string() for c in chain])
try:
    verification.PolicyBuilder().store(store).time(T0 + datetime.timedelta(days=100)).build_server_verifier(
        x509.DNSName("example.com")).verify(leaf, [])
except verification.VerificationError as e:
    print("expired:", str(e)[:80])
csr = (x509.CertificateSigningRequestBuilder().subject_name(x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "csr.example")]))
       .add_extension(x509.SubjectAlternativeName([x509.DNSName("csr.example")]), critical=False).sign(e1, None))
print("csr", h(csr.public_bytes(E.DER)), csr.is_signature_valid, csr.subject.rfc4514_string())
crl = (x509.CertificateRevocationListBuilder().issuer_name(ca_name).last_update(T0).next_update(T0 + datetime.timedelta(days=7))
       .add_revoked_certificate(x509.RevokedCertificateBuilder().serial_number(2002).revocation_date(T0).build())
       .sign(ca_key, hashes.SHA256()))
print("crl", h(crl.public_bytes(E.DER)), crl.is_signature_valid(ca_key.public_key()), [r.serial_number for r in crl])
rsa_cert = x509.CertificateBuilder(ca.issuer, ca.subject, rk.public_key(), 7, T0, T0 + datetime.timedelta(days=1)).sign(rk, hashes.SHA256())
print("rsa-signed cert", h(rsa_cert.public_bytes(E.DER)))
ed_cert = x509.CertificateBuilder(ca.issuer, ca.subject, e4.public_key(), 8, T0, T0 + datetime.timedelta(days=1)).sign(e1, None)
print("ed25519-signed cert", h(ed_cert.public_bytes(E.DER)), ed_cert.signature_algorithm_oid._name, ed_cert.public_key().public_bytes_raw().hex()[:16])

# ── PKCS#7, PKCS#12 ─────────────────────────────────────────────────────────────────────────────
p7 = pkcs7.serialize_certificates([ca, leaf], E.DER)
print("pkcs7 certs", h(p7), [c.serial_number for c in pkcs7.load_der_pkcs7_certificates(p7)])
signed = (pkcs7.PKCS7SignatureBuilder().set_data(b"signed content").add_signer(rsa_cert, rk, hashes.SHA256())
          .sign(E.DER, [pkcs7.PKCS7Options.NoAttributes, pkcs7.PKCS7Options.NoCerts]))
print("pkcs7 signed", h(signed))
enveloped = pkcs7.PKCS7EnvelopeBuilder().set_data(b"for your eyes").add_recipient(rsa_cert).encrypt(E.DER, [])
print("pkcs7 enveloped", pkcs7.pkcs7_decrypt_der(enveloped, rsa_cert, rk, []))
p12 = pkcs12.serialize_key_and_certificates(b"friendly", rk, rsa_cert, [ca], serialization.BestAvailableEncryption(b"p12"))
k, c, extra = pkcs12.load_key_and_certificates(p12, b"p12")
print("pkcs12", c.serial_number, [x.serial_number for x in extra], k.private_numbers() == rk.private_numbers(),
      pkcs12.load_pkcs12(p12, b"p12").cert.friendly_name)

# ── Fernet, HOTP/TOTP, the rest ─────────────────────────────────────────────────────────────────
fkey = b"cw_0x689RpI-jtRR7oE8h_eQsKImvJapLeSbXpwF4e4="
f = Fernet(fkey)
token = f._encrypt_from_parts(b"fernet data", 1700000000, bytes(range(16)))
print("fernet", token, f.decrypt_at_time(token, 100, 1700000050))
try:
    f.decrypt_at_time(token, 10, 1700000050)
except InvalidToken:
    print("fernet expired: InvalidToken")
mf = MultiFernet([Fernet(Fernet.generate_key()), f])
print("multifernet", mf.decrypt(mf.rotate(token)))
otp_key = b"12345678901234567890"
print("hotp", [hotp.HOTP(otp_key, 6, hashes.SHA1()).generate(i) for i in range(5)])
print("totp", totp.TOTP(otp_key, 8, hashes.SHA256(), 30).generate(1111111109),
      totp.TOTP(otp_key, 8, hashes.SHA1(), 30).get_provisioning_uri("alice", "collabo"))
print("constant_time", constant_time.bytes_eq(b"abc", b"abc"), constant_time.bytes_eq(b"abc", b"abd"))
for bad in [lambda: Cipher(algorithms.AES(b"short"), modes.CBC(IV)), lambda: hashes.Hash(hashes.BLAKE2b(32)),
            lambda: aead.AESGCM(b"x" * 15)]:
    try:
        bad()
    except (ValueError, exceptions.UnsupportedAlgorithm) as e:
        print("error", type(e).__name__, e)

# ── threads ─────────────────────────────────────────────────────────────────────────────────────
out = {}


def work(i):
    k = ec.derive_private_key(1000 + i, ec.SECP256R1())
    s = k.sign(DATA, ec.ECDSA(hashes.SHA256(), deterministic_signing=True))
    d = hashes.Hash(hashes.SHA3_256())
    d.update(DATA * (i + 1))
    out[i] = (h(s), d.finalize().hex()[:16], aead.AESGCM(K16).encrypt(IV[:12], bytes([i]) * 100, None).hex()[:16])


ts = [threading.Thread(target=work, args=(i,)) for i in range(6)]
for t in ts:
    t.start()
for t in ts:
    t.join()
for i in sorted(out):
    print("thread", i, *out[i])
print("done")
