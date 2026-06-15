use aes_gcm::aead::{Aead, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Nonce as AesNonce};
use rand::RngCore;
use rsa::pkcs8::LineEnding;
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey};
use rsa::{Oaep, RsaPrivateKey, RsaPublicKey};
use serde_json::{Value, json};
use sha2::Sha256;
use std::io::Write;

use crate::config::Config;
use crate::errors::{MCSError, Result};
use crate::validation;

use pqcrypto_traits::kem::{Ciphertext, PublicKey as KemPublicKey, SecretKey as KemSecretKey, SharedSecret};

// ── Constants ────────────────────────────────────────────

const MAGIC: &[u8] = b"MCPE";
const VERSION: u8 = 1;
const NONCE_SIZE: usize = 12;
const SYMMETRIC_KEY_SIZE: usize = 32;
const HEADER_BASE: usize = 8;

const ALGO_AES256GCM: u8 = 1;
const ALGO_CHACHA20: u8 = 2;
const ALGO_RSA2048: u8 = 3;
const ALGO_RSA4096: u8 = 4;
const ALGO_MLKEM768: u8 = 5;
const ALGO_MLKEM1024: u8 = 6;

// ── Tool: encrypt_file ───────────────────────────────────

pub async fn encrypt_file(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let algorithm = get_opt_str(args, "algorithm").unwrap_or_else(|| "aes-256-gcm".to_string());
    let output = get_opt_str(args, "output");
    let generate = get_opt_bool(args, "generateKey").unwrap_or(false);

    // Extract all argument values before closure
    let key_opt = get_opt_str(args, "key");
    let key_file_opt = get_opt_str(args, "keyFile");
    let pub_key_opt = get_opt_str(args, "publicKey");

    let valid_path = validation::validate_path(&path, &config.allowed_directories, config.server.follow_symlinks)?;
    let metadata = std::fs::metadata(&valid_path)
        .map_err(|e| MCSError::FilesystemError(format!("Cannot read metadata: {e}")))?;
    if !metadata.is_file() {
        return Err(MCSError::InvalidParams(format!("Not a file: {path}")));
    }
    if metadata.len() > config.max_file_size {
        return Err(MCSError::FilesystemError(format!(
            "File size {size} exceeds max {max}", size = metadata.len(), max = config.max_file_size
        )));
    }

    let output_path = resolve_crypto_output(&valid_path, output.as_deref(), &config.allowed_directories, config.server.follow_symlinks)?;
    if output_path == valid_path {
        return Err(MCSError::InvalidParams("Output must differ from source".into()));
    }

    let algo_id = algorithm_to_id(&algorithm)?;
    let file_data = std::fs::read(&valid_path)
        .map_err(|e| MCSError::FilesystemError(format!("Cannot read file: {e}")))?;

    let dst = output_path.clone();

    let result = tokio::task::spawn_blocking(move || -> std::result::Result<EncryptResult, String> {
        match algo_id {
            ALGO_AES256GCM | ALGO_CHACHA20 => {
                let key = resolve_symmetric_key_extracted(key_opt, key_file_opt, generate)?;
                let key_hex = hex::encode(&key);
                let (header, ciphertext) = symmetric_encrypt(&file_data, &key_hex, algo_id)?;
                write_encrypted(&dst, &header, &ciphertext)?;
                Ok(EncryptResult { key: Some(hex::encode(&key)) })
            }
            ALGO_RSA2048 | ALGO_RSA4096 => {
                let bits = if algo_id == ALGO_RSA2048 { 2048 } else { 4096 };
                let pub_pem = pub_key_opt.as_ref()
                    .ok_or_else(|| "Missing required: 'publicKey'".to_string())?;
                let (header, ciphertext, sym_key) = rsa_encrypt(&file_data, pub_pem, bits)?;
                write_encrypted(&dst, &header, &ciphertext)?;
                Ok(EncryptResult { key: Some(hex::encode(&sym_key)) })
            }
            ALGO_MLKEM768 | ALGO_MLKEM1024 => {
                let pub_hex = pub_key_opt.as_ref()
                    .ok_or_else(|| "Missing required: 'publicKey'".to_string())?;
                let pub_bytes = hex::decode(pub_hex)
                    .map_err(|e| format!("Invalid public key hex: {e}"))?;
                let (header, ciphertext, sym_key) = mlkem_encrypt(&file_data, &pub_bytes, algo_id)?;
                write_encrypted(&dst, &header, &ciphertext)?;
                Ok(EncryptResult { key: Some(hex::encode(&sym_key)) })
            }
            _ => Err("Unsupported algorithm".to_string()),
        }
    }).await.map_err(|e| MCSError::FilesystemError(format!("Encrypt task failed: {e}")))?
      .map_err(MCSError::FilesystemError)?;

    let mut resp = json!({
        "success": true,
        "source": valid_path.to_string_lossy(),
        "output": output_path.to_string_lossy(),
        "algorithm": algorithm,
    });
    if let Some(k) = result.key { resp["key"] = json!(k); }
    Ok(resp)
}

// ── Tool: decrypt_file ───────────────────────────────────

pub async fn decrypt_file(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;

    // Extract all argument values before closure
    let key_opt = get_opt_str(args, "key");
    let key_file_opt = get_opt_str(args, "keyFile");
    let priv_key_opt = get_opt_str(args, "privateKey");

    let valid_path = validation::validate_path(&path, &config.allowed_directories, config.server.follow_symlinks)?;
    let file_data = std::fs::read(&valid_path)
        .map_err(|e| MCSError::FilesystemError(format!("Cannot read file: {e}")))?;

    let (algo_id, _enc_key, _nonce, _body) = parse_encrypted(&file_data)
        .map_err(|e| MCSError::InvalidParams(format!("Invalid encrypted file: {e}")))?;
    let algorithm = id_to_algorithm(algo_id)?;
    let output = get_opt_str(args, "output");
    let output_path = resolve_decrypt_output(&valid_path, output.as_deref(), &config.allowed_directories, config.server.follow_symlinks)?;

    if output_path == valid_path {
        return Err(MCSError::InvalidParams("Output must differ from source".into()));
    }

    let result = tokio::task::spawn_blocking(move || -> std::result::Result<Vec<u8>, String> {
        let (_algo_id, enc_key, nonce, body) = parse_encrypted(&file_data)
            .map_err(|e| format!("Parse error: {e}"))?;

        match _algo_id {
            ALGO_AES256GCM | ALGO_CHACHA20 => {
                let key = resolve_symmetric_key_extracted(key_opt, key_file_opt, false)?;
                let key_hex = hex::encode(&key);
                symmetric_decrypt(body, &key_hex, &nonce, _algo_id)
            }
            ALGO_RSA2048 | ALGO_RSA4096 => {
                let priv_pem = priv_key_opt.as_ref()
                    .ok_or_else(|| "Missing required: 'privateKey'".to_string())?;
                rsa_decrypt(body, &enc_key, &nonce, priv_pem)
            }
            ALGO_MLKEM768 | ALGO_MLKEM1024 => {
                let priv_hex = priv_key_opt.as_ref()
                    .ok_or_else(|| "Missing required: 'privateKey'".to_string())?;
                let priv_bytes = hex::decode(priv_hex)
                    .map_err(|e| format!("Invalid private key hex: {e}"))?;
                mlkem_decrypt(body, &enc_key, &nonce, &priv_bytes, _algo_id)
            }
            _ => Err("Unsupported algorithm".into()),
        }
    }).await.map_err(|e| MCSError::FilesystemError(format!("Decrypt task failed: {e}")))?
      .map_err(MCSError::FilesystemError)?;

    std::fs::write(&output_path, &result)
        .map_err(|e| MCSError::FilesystemError(format!("Cannot write output: {e}")))?;

    Ok(json!({
        "success": true,
        "source": valid_path.to_string_lossy(),
        "output": output_path.to_string_lossy(),
        "algorithm": algorithm,
        "decryptedSize": result.len(),
    }))
}

// ── Tool: generate_key ───────────────────────────────────

pub async fn generate_key(args: Option<&Value>, config: &Config) -> Result<Value> {
    let algorithm = get_opt_str(args, "algorithm").unwrap_or_else(|| "aes-256".to_string());
    let output = get_opt_str(args, "output");

    // Validate output path before doing any work
    if let Some(ref out) = output {
        validation::validate_destination(out, &config.allowed_directories, config.server.follow_symlinks)?;
    }

    let alg_clone = algorithm.clone();
    let out_clone = output.clone();

    let result = tokio::task::spawn_blocking(move || -> std::result::Result<KeygenResult, String> {
        match alg_clone.as_str() {
            "aes-256" => {
                let mut key = vec![0u8; SYMMETRIC_KEY_SIZE];
                OsRng.fill_bytes(&mut key);
                let key_hex = hex::encode(&key);
                if let Some(ref out) = out_clone {
                    std::fs::write(out, &key_hex)
                        .map_err(|e| format!("Cannot write key file: {e}"))?;
                }
                Ok(KeygenResult { key: Some(key_hex), public_key: None, private_key: None })
            }
            "rsa-2048" | "rsa-4096" => {
                let bits: usize = if alg_clone == "rsa-2048" { 2048 } else { 4096 };
                let mut rng = OsRng;
                let priv_key = RsaPrivateKey::new(&mut rng, bits)
                    .map_err(|e| format!("RSA key generation failed: {e}"))?;
                let pub_key = RsaPublicKey::from(&priv_key);
                let priv_pem = priv_key.to_pkcs8_pem(LineEnding::LF)
                    .map_err(|e| format!("PEM encoding failed: {e}"))?.to_string();
                let pub_pem = pub_key.to_public_key_pem(LineEnding::LF)
                    .map_err(|e| format!("PEM encoding failed: {e}"))?;
                if let Some(ref out) = out_clone {
                    std::fs::write(format!("{out}.pub"), &pub_pem)
                        .map_err(|e| format!("Cannot write public key: {e}"))?;
                    std::fs::write(out, &priv_pem)
                        .map_err(|e| format!("Cannot write private key: {e}"))?;
                }
                Ok(KeygenResult { key: None, public_key: Some(pub_pem), private_key: Some(priv_pem) })
            }
            "kyber-768" | "kyber-1024" | "mlkem-768" | "mlkem-1024" => {
                let (_pk, _sk, pk_hex, sk_hex) = if alg_clone == "kyber-768" || alg_clone == "mlkem-768" {
                    let (pk, sk) = pqcrypto_mlkem::mlkem768::keypair();
                    (pk.as_bytes().to_vec(), sk.as_bytes().to_vec(),
                     hex::encode(pk.as_bytes()), hex::encode(sk.as_bytes()))
                } else {
                    let (pk, sk) = pqcrypto_mlkem::mlkem1024::keypair();
                    (pk.as_bytes().to_vec(), sk.as_bytes().to_vec(),
                     hex::encode(pk.as_bytes()), hex::encode(sk.as_bytes()))
                };
                if let Some(ref out) = out_clone {
                    std::fs::write(out, &sk_hex)
                        .map_err(|e| format!("Cannot write secret key: {e}"))?;
                    std::fs::write(format!("{out}.pub"), &pk_hex)
                        .map_err(|e| format!("Cannot write public key: {e}"))?;
                }
                Ok(KeygenResult { key: None, public_key: Some(pk_hex), private_key: Some(sk_hex) })
            }
            _ => Err(format!("Unsupported key algorithm: {alg_clone}")),
        }
    }).await.map_err(|e| MCSError::FilesystemError(format!("Key generation failed: {e}")))?
      .map_err(MCSError::FilesystemError)?;

    let mut resp = json!({ "success": true, "algorithm": algorithm });
    if let Some(k) = result.key { resp["key"] = json!(k); }
    if let Some(pk) = result.public_key { resp["publicKey"] = json!(pk); }
    if let Some(sk) = result.private_key { resp["privateKey"] = json!(sk); }
    if let Some(ref out) = output {
        if algorithm == "aes-256" {
            resp["keyFile"] = json!(out);
        } else {
            resp["privateKeyFile"] = json!(out);
            resp["publicKeyFile"] = json!(format!("{out}.pub"));
        }
    }
    Ok(resp)
}

// ── Encryption / Decryption Helpers ──────────────────────

#[derive(Default)]
struct EncryptResult {
    key: Option<String>,
}

struct KeygenResult {
    key: Option<String>,
    public_key: Option<String>,
    private_key: Option<String>,
}

fn symmetric_encrypt(
    plaintext: &[u8],
    key_hex: &str,
    algo_id: u8,
) -> std::result::Result<(Vec<u8>, Vec<u8>), String> {
    let key_bytes = hex::decode(key_hex)
        .map_err(|e| format!("Invalid key hex: {e}"))?;
    if key_bytes.len() != SYMMETRIC_KEY_SIZE {
        return Err(format!("Key must be {SYMMETRIC_KEY_SIZE} bytes"));
    }

    let mut nonce = [0u8; NONCE_SIZE];
    OsRng.fill_bytes(&mut nonce);

    let header = build_header(algo_id, &[]);
    let aad = &header[..header.len() - 2];

    let ciphertext = if algo_id == ALGO_CHACHA20 {
        use chacha20poly1305::aead::Aead as _;
        let key = chacha20poly1305::Key::from_slice(&key_bytes);
        let cipher = chacha20poly1305::ChaCha20Poly1305::new(key);
        let nonce_arr = chacha20poly1305::Nonce::from_slice(&nonce);
        cipher.encrypt(nonce_arr, Payload { msg: plaintext, aad })
            .map_err(|e| format!("ChaCha20 encryption failed: {e}"))?
    } else {
        let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(key);
        cipher.encrypt(AesNonce::from_slice(&nonce), Payload { msg: plaintext, aad })
            .map_err(|e| format!("AES-256-GCM encryption failed: {e}"))?
    };

    Ok(([header, nonce.to_vec()].concat(), ciphertext))
}

fn symmetric_decrypt(
    ciphertext: &[u8],
    key_hex: &str,
    nonce: &[u8],
    algo_id: u8,
) -> std::result::Result<Vec<u8>, String> {
    let key_bytes = hex::decode(key_hex)
        .map_err(|e| format!("Invalid key hex: {e}"))?;
    if key_bytes.len() != SYMMETRIC_KEY_SIZE {
        return Err(format!("Key must be {SYMMETRIC_KEY_SIZE} bytes"));
    }

    let header = build_header(algo_id, &[]);
    let aad = &header[..header.len() - 2];

    let plaintext = if algo_id == ALGO_CHACHA20 {
        use chacha20poly1305::aead::Aead as _;
        let key = chacha20poly1305::Key::from_slice(&key_bytes);
        let cipher = chacha20poly1305::ChaCha20Poly1305::new(key);
        let nonce_arr = chacha20poly1305::Nonce::from_slice(nonce);
        cipher.decrypt(nonce_arr, Payload { msg: ciphertext, aad })
            .map_err(|e| format!("ChaCha20 decryption failed (wrong key or data): {e}"))?
    } else {
        let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(key);
        cipher.decrypt(AesNonce::from_slice(nonce), Payload { msg: ciphertext, aad })
            .map_err(|e| format!("AES-256-GCM decryption failed (wrong key or data): {e}"))?
    };

    Ok(plaintext)
}

#[allow(clippy::type_complexity)]
fn rsa_encrypt(
    plaintext: &[u8],
    pub_key_pem: &str,
    bits: usize,
) -> std::result::Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
    let pub_key = RsaPublicKey::from_public_key_pem(pub_key_pem)
        .map_err(|e| format!("Invalid RSA public key PEM: {e}"))?;

    let mut sym_key = vec![0u8; SYMMETRIC_KEY_SIZE];
    OsRng.fill_bytes(&mut sym_key);

    let mut rng = OsRng;
    let enc_key = pub_key.encrypt(&mut rng, Oaep::new::<Sha256>(), &sym_key)
        .map_err(|e| format!("RSA encryption failed: {e}"))?;

    let mut nonce = [0u8; NONCE_SIZE];
    OsRng.fill_bytes(&mut nonce);

    let algo_id = if bits == 2048 { ALGO_RSA2048 } else { ALGO_RSA4096 };
    let header = build_header(algo_id, &enc_key);
    let aad = &header[..header.len() - 2];

    let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&sym_key);
    let cipher = Aes256Gcm::new(key);
    let ciphertext = cipher.encrypt(AesNonce::from_slice(&nonce), Payload { msg: plaintext, aad })
        .map_err(|e| format!("AES encryption failed: {e}"))?;

    Ok(([header, nonce.to_vec()].concat(), ciphertext, sym_key))
}

fn rsa_decrypt(
    ciphertext: &[u8],
    enc_key: &[u8],
    nonce: &[u8],
    priv_key_pem: &str,
) -> std::result::Result<Vec<u8>, String> {
    let priv_key = RsaPrivateKey::from_pkcs8_pem(priv_key_pem)
        .map_err(|e| format!("Invalid RSA private key PEM: {e}"))?;

    let sym_key = priv_key.decrypt(Oaep::new::<Sha256>(), enc_key)
        .map_err(|e| format!("RSA decryption failed: {e}"))?;

    let algo_id = if enc_key.len() == 256 { ALGO_RSA2048 } else { ALGO_RSA4096 };
    let header = build_header(algo_id, enc_key);
    let aad = &header[..header.len() - 2];

    let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&sym_key);
    let cipher = Aes256Gcm::new(key);
    let plaintext = cipher.decrypt(AesNonce::from_slice(nonce), Payload { msg: ciphertext, aad })
        .map_err(|e| format!("AES decryption failed: {e}"))?;

    Ok(plaintext)
}

#[allow(clippy::type_complexity)]
fn mlkem_encrypt(
    plaintext: &[u8],
    pub_key: &[u8],
    algo_id: u8,
) -> std::result::Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
    let (shared_secret, ct) = if algo_id == ALGO_MLKEM768 {
        let pk = pqcrypto_mlkem::mlkem768::PublicKey::from_bytes(pub_key)
            .map_err(|e| format!("Invalid ML-KEM-768 public key: {e}"))?;
        let (ss, c) = pqcrypto_mlkem::mlkem768::encapsulate(&pk);
        (ss.as_bytes().to_vec(), c.as_bytes().to_vec())
    } else {
        let pk = pqcrypto_mlkem::mlkem1024::PublicKey::from_bytes(pub_key)
            .map_err(|e| format!("Invalid ML-KEM-1024 public key: {e}"))?;
        let (ss, c) = pqcrypto_mlkem::mlkem1024::encapsulate(&pk);
        (ss.as_bytes().to_vec(), c.as_bytes().to_vec())
    };

    let mut nonce = [0u8; NONCE_SIZE];
    OsRng.fill_bytes(&mut nonce);

    let header = build_header(algo_id, &ct);
    let aad = &header[..header.len() - 2];

    let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&shared_secret);
    let cipher = Aes256Gcm::new(key);
    let ciphertext = cipher.encrypt(AesNonce::from_slice(&nonce), Payload { msg: plaintext, aad })
        .map_err(|e| format!("AES encryption failed: {e}"))?;

    Ok(([header, nonce.to_vec()].concat(), ciphertext, shared_secret))
}

fn mlkem_decrypt(
    ciphertext: &[u8],
    enc_key: &[u8],
    nonce: &[u8],
    priv_key: &[u8],
    algo_id: u8,
) -> std::result::Result<Vec<u8>, String> {
    let shared_secret = if algo_id == ALGO_MLKEM768 {
        let sk = pqcrypto_mlkem::mlkem768::SecretKey::from_bytes(priv_key)
            .map_err(|e| format!("Invalid ML-KEM-768 secret key: {e}"))?;
        let ct = pqcrypto_mlkem::mlkem768::Ciphertext::from_bytes(enc_key)
            .map_err(|e| format!("Invalid ML-KEM ciphertext: {e}"))?;
        pqcrypto_mlkem::mlkem768::decapsulate(&ct, &sk).as_bytes().to_vec()
    } else {
        let sk = pqcrypto_mlkem::mlkem1024::SecretKey::from_bytes(priv_key)
            .map_err(|e| format!("Invalid ML-KEM-1024 secret key: {e}"))?;
        let ct = pqcrypto_mlkem::mlkem1024::Ciphertext::from_bytes(enc_key)
            .map_err(|e| format!("Invalid ML-KEM ciphertext: {e}"))?;
        pqcrypto_mlkem::mlkem1024::decapsulate(&ct, &sk).as_bytes().to_vec()
    };

    let header = build_header(algo_id, enc_key);
    let aad = &header[..header.len() - 2];

    let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&shared_secret);
    let cipher = Aes256Gcm::new(key);
    let plaintext = cipher.decrypt(AesNonce::from_slice(nonce), Payload { msg: ciphertext, aad })
        .map_err(|e| format!("AES decryption failed: {e}"))?;

    Ok(plaintext)
}

// ── File Format Helpers ──────────────────────────────────

fn build_header(algo_id: u8, enc_key: &[u8]) -> Vec<u8> {
    let key_len = enc_key.len() as u16;
    let mut header = Vec::with_capacity(HEADER_BASE + enc_key.len());
    header.extend_from_slice(MAGIC);
    header.push(VERSION);
    header.push(algo_id);
    header.extend_from_slice(&key_len.to_be_bytes());
    header.extend_from_slice(enc_key);
    header
}

#[allow(clippy::type_complexity)]
fn parse_encrypted(data: &[u8]) -> std::result::Result<(u8, Vec<u8>, Vec<u8>, &[u8]), String> {
    if data.len() < HEADER_BASE + NONCE_SIZE + 1 {
        return Err("File too small to be a valid encrypted file".into());
    }
    if &data[..4] != MAGIC {
        return Err("Invalid magic bytes — not an MCPE encrypted file".into());
    }
    if data[4] != VERSION {
        return Err(format!("Unsupported version: {}", data[4]));
    }

    let algo_id = data[5];
    let key_len = u16::from_be_bytes([data[6], data[7]]) as usize;
    let header_total = HEADER_BASE + key_len;

    if data.len() < header_total + NONCE_SIZE + 1 {
        return Err("File truncated".into());
    }

    let enc_key = data[HEADER_BASE..header_total].to_vec();
    let nonce = data[header_total..header_total + NONCE_SIZE].to_vec();
    let body = &data[header_total + NONCE_SIZE..];

    Ok((algo_id, enc_key, nonce, body))
}

fn write_encrypted(path: &std::path::Path, header: &[u8], ciphertext: &[u8]) -> std::result::Result<(), String> {
    let mut file = std::fs::File::create(path)
        .map_err(|e| format!("Cannot create output: {e}"))?;
    file.write_all(header)
        .map_err(|e| format!("Cannot write header: {e}"))?;
    file.write_all(ciphertext)
        .map_err(|e| format!("Cannot write ciphertext: {e}"))?;
    file.flush().map_err(|e| format!("Cannot flush: {e}"))?;
    Ok(())
}

// ── Key Resolution ───────────────────────────────────────

fn resolve_symmetric_key_extracted(
    key_opt: Option<String>,
    key_file_opt: Option<String>,
    generate: bool,
) -> std::result::Result<Vec<u8>, String> {
    if let Some(k) = key_opt {
        let bytes = hex::decode(&k).map_err(|e| format!("Invalid key hex: {e}"))?;
        if bytes.len() != SYMMETRIC_KEY_SIZE {
            return Err(format!("Key must be {SYMMETRIC_KEY_SIZE} hex bytes"));
        }
        return Ok(bytes);
    }
    if let Some(kf) = key_file_opt {
        let content = std::fs::read_to_string(&kf)
            .map_err(|e| format!("Cannot read key file: {e}"))?;
        let trimmed = content.trim().to_string();
        let bytes = hex::decode(&trimmed).map_err(|e| format!("Invalid key hex in file: {e}"))?;
        if bytes.len() != SYMMETRIC_KEY_SIZE {
            return Err(format!("Key must be {SYMMETRIC_KEY_SIZE} hex bytes"));
        }
        return Ok(bytes);
    }
    if generate {
        let mut key = vec![0u8; SYMMETRIC_KEY_SIZE];
        OsRng.fill_bytes(&mut key);
        return Ok(key);
    }
    Err("No key provided. Use 'key', 'keyFile', or 'generateKey: true'".into())
}

// ── Algorithm Helpers ────────────────────────────────────

fn algorithm_to_id(name: &str) -> Result<u8> {
    Ok(match name {
        "aes-256-gcm" => ALGO_AES256GCM,
        "chacha20-poly1305" => ALGO_CHACHA20,
        "rsa-2048-oaep" => ALGO_RSA2048,
        "rsa-4096-oaep" => ALGO_RSA4096,
        "kyber-768" | "mlkem-768" => ALGO_MLKEM768,
        "kyber-1024" | "mlkem-1024" => ALGO_MLKEM1024,
        _ => return Err(MCSError::InvalidParams(format!("Unsupported algorithm: {name}"))),
    })
}

fn id_to_algorithm(id: u8) -> Result<String> {
    Ok(match id {
        ALGO_AES256GCM => "aes-256-gcm".into(),
        ALGO_CHACHA20 => "chacha20-poly1305".into(),
        ALGO_RSA2048 => "rsa-2048-oaep".into(),
        ALGO_RSA4096 => "rsa-4096-oaep".into(),
        ALGO_MLKEM768 => "mlkem-768".into(),
        ALGO_MLKEM1024 => "mlkem-1024".into(),
        _ => return Err(MCSError::InvalidParams(format!("Unknown algorithm ID: {id}"))),
    })
}

fn resolve_crypto_output(
    source: &std::path::Path,
    explicit: Option<&str>,
    allowed_dirs: &[String],
    follow_symlinks: bool,
) -> Result<std::path::PathBuf> {
    if let Some(out) = explicit {
        validation::validate_destination(out, allowed_dirs, follow_symlinks)
    } else {
        let mut result = source.to_path_buf();
        let name = result.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        result.set_file_name(format!("{name}.enc"));
        Ok(result)
    }
}

fn resolve_decrypt_output(
    source: &std::path::Path,
    explicit: Option<&str>,
    allowed_dirs: &[String],
    follow_symlinks: bool,
) -> Result<std::path::PathBuf> {
    if let Some(out) = explicit {
        validation::validate_destination(out, allowed_dirs, follow_symlinks)
    } else {
        let name = source.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let stripped = name.strip_suffix(".enc").unwrap_or(&name);
        let mut result = source.to_path_buf();
        result.set_file_name(stripped);
        Ok(result)
    }
}

// ── Argument Helpers ─────────────────────────────────────

fn get_str_arg(args: Option<&Value>, name: &str) -> Result<String> {
    args.and_then(|a| a.get(name))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| MCSError::InvalidParams(format!("Missing required: '{name}'")))
}

fn get_opt_str(args: Option<&Value>, name: &str) -> Option<String> {
    args.and_then(|a| a.get(name))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn get_opt_bool(args: Option<&Value>, name: &str) -> Option<bool> {
    args.and_then(|a| a.get(name)).and_then(|v| v.as_bool())
}
