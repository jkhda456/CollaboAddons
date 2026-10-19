//! User authentication (RFC 4252): publickey (with key files and the ssh-agent),
//! keyboard-interactive and password, tried in the order the methods are offered and configured.

use super::agent::{self, Agent};
use super::hostkey::PublicKey;
use super::privatekey::Identity;
use super::transport::Transport;
use super::wire::{Reader, Writer};
use super::msg;
use crate::{err, Result};

/// How the agent answers a keyboard-interactive prompt.
pub type Prompter<'a> = Box<dyn FnMut(&str, &str, &[(String, bool)]) -> Vec<String> + 'a>;

/// What we try to authenticate with.
pub struct Credentials<'a> {
    pub user: String,
    pub password: Option<String>,
    pub identities: Vec<Identity>,
    pub agent: Option<Agent>,
    pub prompter: Option<Prompter<'a>>,
    /// Try the agent and key files (publickey) at all.
    pub allow_publickey: bool,
    /// How authentication succeeded (method and key), once it has.
    pub used: Option<String>,
}

impl Credentials<'_> {
    pub fn new(user: &str) -> Credentials<'static> {
        Credentials { user: user.to_string(), password: None, identities: Vec::new(), agent: None, prompter: None, allow_publickey: true, used: None }
    }
}

/// Runs authentication to the end, or returns why it failed.
pub fn authenticate(t: &mut Transport, creds: &mut Credentials) -> Result<()> {
    // Ask for the user-auth service.
    let mut w = Writer::msg(msg::SERVICE_REQUEST);
    w.string("ssh-userauth");
    t.send_packet(&w.finish())?;
    let reply = t.recv_packet()?;
    if reply.first() != Some(&msg::SERVICE_ACCEPT) {
        return Err(err!("the server would not start user authentication"));
    }

    // A "none" request to learn which methods the server will accept.
    let mut methods = match try_none(t, &creds.user)? {
        Auth::Success => {
            creds.used = Some("none (the server asked for nothing)".into());
            return Ok(());
        }
        Auth::Failure { methods, .. } => methods,
    };

    // Each method once (a partial success — the server wants another method too — opens the
    // others again); the server's latest list of methods decides what is left.
    let mut tried: Vec<&'static str> = Vec::new();
    let mut last_error = String::from("the server accepted none of the methods tried");
    loop {
        let offers = |methods: &[String], m: &str| methods.iter().any(|x| x == m);
        let mut outcome = None;
        if creds.allow_publickey && offers(&methods, "publickey") && !tried.contains(&"publickey") {
            tried.push("publickey");
            last_error = "no public key was accepted (from the ssh-agent or key files)".into();
            outcome = Some(try_publickey(t, creds, &methods)?);
        } else if offers(&methods, "keyboard-interactive")
            && (creds.password.is_some() || creds.prompter.is_some())
            && !tried.contains(&"keyboard-interactive")
        {
            tried.push("keyboard-interactive");
            last_error = "keyboard-interactive was refused".into();
            outcome = Some(try_keyboard(t, creds)?);
            if matches!(outcome, Some(Auth::Success)) {
                creds.used = Some("keyboard-interactive".into());
            }
        } else if offers(&methods, "password") && creds.password.is_some() && !tried.contains(&"password") {
            tried.push("password");
            last_error = "the password was refused".into();
            outcome = Some(try_password(t, &creds.user, creds.password.as_deref().unwrap())?);
            if matches!(outcome, Some(Auth::Success)) {
                creds.used = Some("password".into());
            }
        }
        match outcome {
            Some(Auth::Success) => return Ok(()),
            Some(Auth::Failure { methods: m, partial }) => {
                if partial {
                    let last = tried.last().copied();
                    tried.clear();
                    tried.extend(last);
                }
                methods = m;
            }
            None => {
                let hint = if creds.password.is_none() && (offers(&methods, "password") || offers(&methods, "keyboard-interactive")) {
                    "; a password would be tried with --password-env or --password-file"
                } else {
                    ""
                };
                return Err(err!("{last_error} (the server accepts: {}){hint}", methods.join(", ")));
            }
        }
    }
}

enum Auth {
    Success,
    Failure { methods: Vec<String>, partial: bool },
}

/// Reads an auth reply, skipping banners. Returns success, or the methods that may continue.
fn read_outcome(t: &mut Transport) -> Result<Auth> {
    loop {
        let p = t.recv_packet()?;
        match p.first().copied() {
            Some(msg::USERAUTH_SUCCESS) => return Ok(Auth::Success),
            Some(msg::USERAUTH_BANNER) => continue,
            Some(msg::USERAUTH_FAILURE) => {
                let mut r = Reader::new(&p[1..]);
                let methods = r.name_list()?;
                let partial = r.bool().unwrap_or(false);
                return Ok(Auth::Failure { methods, partial });
            }
            Some(other) => return Err(err!("unexpected message {other} during authentication")),
            None => return Err(err!("an empty authentication reply")),
        }
    }
}

fn try_none(t: &mut Transport, user: &str) -> Result<Auth> {
    let mut w = Writer::msg(msg::USERAUTH_REQUEST);
    w.string(user).string("ssh-connection").string("none");
    t.send_packet(&w.finish())?;
    read_outcome(t)
}

fn try_password(t: &mut Transport, user: &str, password: &str) -> Result<Auth> {
    let mut w = Writer::msg(msg::USERAUTH_REQUEST);
    w.string(user).string("ssh-connection").string("password").bool(false).string(password);
    t.send_packet(&w.finish())?;
    loop {
        let p = t.recv_packet()?;
        if p.first() == Some(&msg::USERAUTH_PASSWD_CHANGEREQ) {
            // We cannot change an expired password; report it rather than hang.
            return Err(err!("the server wants the password changed, which c-sh cannot do"));
        }
        return match p.first().copied() {
            Some(msg::USERAUTH_SUCCESS) => Ok(Auth::Success),
            Some(msg::USERAUTH_BANNER) => continue,
            Some(msg::USERAUTH_FAILURE) => {
                let mut r = Reader::new(&p[1..]);
                Ok(Auth::Failure { methods: r.name_list()?, partial: r.bool().unwrap_or(false) })
            }
            Some(other) => Err(err!("unexpected message {other} answering a password")),
            None => Err(err!("an empty password reply")),
        };
    }
}

/// The signature algorithms to try for a key, best first, given what the server accepts.
fn algorithms_for(key: &PublicKey, server_sig_algs: &[String]) -> Vec<String> {
    let all = key.algorithms();
    if matches!(key, PublicKey::Rsa { .. }) && !server_sig_algs.is_empty() {
        let mut ordered: Vec<String> = all.into_iter().filter(|a| server_sig_algs.contains(a)).collect();
        if ordered.is_empty() {
            // The server named none of ours; fall back to the modern default and hope.
            ordered.push("rsa-sha2-512".into());
        }
        ordered
    } else {
        all
    }
}

/// The data a publickey signature covers (RFC 4252 §7).
fn signed_data(session_id: &[u8], user: &str, alg: &str, key_blob: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.string(session_id);
    w.u8(msg::USERAUTH_REQUEST);
    w.string(user).string("ssh-connection").string("publickey").bool(true).string(alg).string(key_blob);
    w.finish()
}

fn publickey_request(user: &str, alg: &str, key_blob: &[u8], signature: Option<&[u8]>) -> Vec<u8> {
    let mut w = Writer::msg(msg::USERAUTH_REQUEST);
    w.string(user).string("ssh-connection").string("publickey").bool(signature.is_some()).string(alg).string(key_blob);
    if let Some(sig) = signature {
        w.string(sig);
    }
    w.finish()
}

/// One candidate key: the agent's (signed by it) or a key file's.
enum Signer<'a> {
    Agent,
    File(&'a Identity),
}

fn try_publickey(t: &mut Transport, creds: &mut Credentials, methods: &[String]) -> Result<Auth> {
    let user = creds.user.clone();
    let session_id = t.session_id().to_vec();
    let server_algs = t.server_sig_algs.clone();
    let mut last_methods: Vec<String> = methods.to_vec();

    // The agent's keys first (in the sandbox the host holds them), then the key files.
    let agent_keys = creds.agent.as_mut().map(|a| a.identities().unwrap_or_default()).unwrap_or_default();
    let mut candidates: Vec<(PublicKey, Vec<u8>, String, Signer)> = Vec::new();
    for k in &agent_keys {
        candidates.push((k.key.clone(), k.blob.clone(), format!("from the ssh-agent ({})", k.comment), Signer::Agent));
    }
    for id in &creds.identities {
        let what = if id.comment.is_empty() { "key file".to_string() } else { id.comment.clone() };
        candidates.push((id.public.clone(), id.public.blob(), format!("({what})"), Signer::File(id)));
    }

    for (key, blob, what, signer) in &candidates {
        for alg in algorithms_for(key, &server_algs) {
            // Ask first whether the server would take this key (no signature yet).
            match query_key(t, &user, &alg, blob)? {
                Probe::Success => {
                    creds.used = Some(format!("publickey {alg} {what}"));
                    return Ok(Auth::Success);
                }
                Probe::Rejected(m) => {
                    let still = m.iter().any(|x| x == "publickey");
                    last_methods = m;
                    if !still {
                        return Ok(Auth::Failure { methods: last_methods, partial: false });
                    }
                    continue; // another algorithm for this key, or the next key
                }
                Probe::Acceptable => {}
            }
            let data = signed_data(&session_id, &user, &alg, blob);
            let signed = match signer {
                Signer::Agent => {
                    let flags = match alg.as_str() {
                        "rsa-sha2-256" => agent::RSA_SHA2_256,
                        "rsa-sha2-512" => agent::RSA_SHA2_512,
                        _ => 0,
                    };
                    creds.agent.as_mut().map(|a| a.sign(blob, &data, flags))
                }
                Signer::File(id) => Some(id.sign(&data, &alg)),
            };
            let Some(Ok(sig)) = signed else { break }; // could not sign with it: the next key
            t.send_packet(&publickey_request(&user, &alg, blob, Some(&sig)))?;
            match read_outcome(t)? {
                Auth::Success => {
                    creds.used = Some(format!("publickey {alg} {what}"));
                    return Ok(Auth::Success);
                }
                Auth::Failure { methods: m, partial } => {
                    let still = m.iter().any(|x| x == "publickey");
                    if partial || !still {
                        return Ok(Auth::Failure { methods: m, partial });
                    }
                    last_methods = m;
                    break; // this key's signature was refused: the next key
                }
            }
        }
    }
    Ok(Auth::Failure { methods: last_methods, partial: false })
}

enum Probe {
    /// PK_OK: sign with this key and algorithm.
    Acceptable,
    /// Not this one; the methods the server still accepts.
    Rejected(Vec<String>),
    /// The server let us in on the probe alone (unusual, allowed).
    Success,
}

/// A publickey probe (has_sig=false, RFC 4252 §7).
fn query_key(t: &mut Transport, user: &str, alg: &str, blob: &[u8]) -> Result<Probe> {
    t.send_packet(&publickey_request(user, alg, blob, None))?;
    loop {
        let p = t.recv_packet()?;
        return Ok(match p.first().copied() {
            Some(msg::USERAUTH_PK_OK) => Probe::Acceptable,
            Some(msg::USERAUTH_BANNER) => continue,
            Some(msg::USERAUTH_SUCCESS) => Probe::Success,
            Some(msg::USERAUTH_FAILURE) => Probe::Rejected(Reader::new(&p[1..]).name_list()?),
            Some(other) => return Err(err!("unexpected message {other} probing a public key")),
            None => return Err(err!("an empty public-key probe reply")),
        });
    }
}

fn try_keyboard(t: &mut Transport, creds: &mut Credentials) -> Result<Auth> {
    let mut w = Writer::msg(msg::USERAUTH_REQUEST);
    w.string(&creds.user).string("ssh-connection").string("keyboard-interactive").string("").string("");
    t.send_packet(&w.finish())?;
    loop {
        let p = t.recv_packet()?;
        match p.first().copied() {
            Some(msg::USERAUTH_SUCCESS) => return Ok(Auth::Success),
            Some(msg::USERAUTH_BANNER) => continue,
            Some(msg::USERAUTH_FAILURE) => {
                let mut r = Reader::new(&p[1..]);
                return Ok(Auth::Failure { methods: r.name_list()?, partial: r.bool().unwrap_or(false) });
            }
            Some(msg::USERAUTH_INFO_REQUEST) => {
                let mut r = Reader::new(&p[1..]);
                let name = r.text()?;
                let instruction = r.text()?;
                let _lang = r.text()?;
                let count = r.u32()?;
                let mut prompts = Vec::new();
                for _ in 0..count {
                    let prompt = r.text()?;
                    let echo = r.bool()?;
                    prompts.push((prompt, echo));
                }
                let answers = answer_prompts(creds, &name, &instruction, &prompts);
                let mut resp = Writer::msg(msg::USERAUTH_INFO_RESPONSE);
                resp.u32(answers.len() as u32);
                for a in &answers {
                    resp.string(a);
                }
                t.send_packet(&resp.finish())?;
            }
            Some(other) => return Err(err!("unexpected message {other} during keyboard-interactive")),
            None => return Err(err!("an empty keyboard-interactive reply")),
        }
    }
}

/// Answers the prompts: the configured prompter, else the password for a single hidden prompt.
fn answer_prompts(creds: &mut Credentials, name: &str, instruction: &str, prompts: &[(String, bool)]) -> Vec<String> {
    if let Some(p) = creds.prompter.as_mut() {
        return p(name, instruction, prompts);
    }
    if prompts.len() == 1 && !prompts[0].1 {
        if let Some(pw) = &creds.password {
            return vec![pw.clone()];
        }
    }
    prompts.iter().map(|_| String::new()).collect()
}
