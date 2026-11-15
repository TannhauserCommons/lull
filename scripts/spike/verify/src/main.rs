//! `spike-verify <public key> <file> <signature>`: the key and the signature are the files Tauri writes (base64 of
//! minisign's own files). Prints `hub=ok|FAIL` (the steps of crates/server/src/kit.rs `signed_by`, which refuses
//! legacy signatures) and `updater=ok|FAIL` (the steps of tauri-plugin-updater 2.14.1 `verify_signature`, which allows
//! them), and the signature's trusted comment. Exit 2 when it could not read its inputs.
use std::{env, fs, process::ExitCode};

use base64::Engine;
use minisign_verify::{PublicKey, Signature};

fn decode(b64: &str) -> Option<String> {
    base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let [_, key, file, sig] = &args[..] else {
        eprintln!("usage: spike-verify <public key> <file> <signature>");
        return ExitCode::from(2);
    };
    let (Ok(key), Ok(file), Ok(sig)) = (fs::read_to_string(key), fs::read(file), fs::read_to_string(sig)) else {
        eprintln!("spike-verify: cannot read an input");
        return ExitCode::from(2);
    };
    let (Some(key), Some(sig)) = (decode(&key), decode(&sig)) else {
        println!("hub=FAIL\nupdater=FAIL\ncomment=(not base64)");
        return ExitCode::SUCCESS;
    };
    let (Ok(key), Ok(sig)) = (PublicKey::decode(&key), Signature::decode(&sig)) else {
        println!("hub=FAIL\nupdater=FAIL\ncomment=(not a minisign key or signature)");
        return ExitCode::SUCCESS;
    };
    let word = |ok: bool| if ok { "ok" } else { "FAIL" };
    println!("hub={}", word(key.verify(&file, &sig, false).is_ok()));
    println!("updater={}", word(key.verify(&file, &sig, true).is_ok()));
    println!("comment={}", sig.trusted_comment());
    ExitCode::SUCCESS
}
