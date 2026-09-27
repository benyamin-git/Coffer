use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::format::{FormatError, HEADER_LEN, Header, KdfParams, NONCE_LEN, SALT_LEN, TAG_LEN};

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error(transparent)]
    Format(#[from] FormatError),
    #[error("decryption failed (wrong password or corrupted vault)")]
    Decrypt,
    #[error("encryption failed")]
    Encrypt,
    #[error("key derivation failed")]
    Kdf,
    #[error("failed to gather randomness from the operating system")]
    Random,
}

fn random_bytes(buf: &mut [u8]) -> Result<(), CryptoError> {
    getrandom::fill(buf).map_err(|_| CryptoError::Random)
}

fn derive_key(
    password: &[u8],
    params: &KdfParams,
    salt: &[u8],
) -> Result<Zeroizing<[u8; 32]>, CryptoError> {
    let params = Params::new(params.m_cost, params.t_cost, params.p_cost, Some(32))
        .map_err(|_| CryptoError::Kdf)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon2
        .hash_password_into(password, salt, key.as_mut())
        .map_err(|_| CryptoError::Kdf)?;
    Ok(key)
}

fn cipher_for(password: &[u8], header: &Header) -> Result<XChaCha20Poly1305, CryptoError> {
    let key = derive_key(password, &header.params, &header.salt)?;
    XChaCha20Poly1305::new_from_slice(&key[..]).map_err(|_| CryptoError::Kdf)
}

pub fn seal_with_params(
    password: &[u8],
    plaintext: &[u8],
    params: KdfParams,
) -> Result<Vec<u8>, CryptoError> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    random_bytes(&mut salt)?;
    random_bytes(&mut nonce)?;

    let header = Header {
        params,
        salt,
        nonce,
    };
    let header_bytes = header.encode();
    let cipher = cipher_for(password, &header)?;
    let nonce = XNonce::try_from(&header.nonce[..]).map_err(|_| CryptoError::Encrypt)?;
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad: &header_bytes,
            },
        )
        .map_err(|_| CryptoError::Encrypt)?;

    let mut out = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    out.extend_from_slice(&header_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

pub fn seal(password: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    seal_with_params(password, plaintext, KdfParams::default())
}

pub struct Opened {
    pub plaintext: Zeroizing<Vec<u8>>,
    pub params: KdfParams,
}

impl std::fmt::Debug for Opened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opened")
            .field("plaintext", &"<redacted>")
            .field("params", &self.params)
            .finish()
    }
}

pub fn open(password: &[u8], data: &[u8]) -> Result<Opened, CryptoError> {
    let (header, ciphertext) = Header::parse(data)?;
    if ciphertext.len() < TAG_LEN {
        return Err(CryptoError::Decrypt);
    }
    let cipher = cipher_for(password, &header)?;
    let nonce = XNonce::try_from(&header.nonce[..]).map_err(|_| CryptoError::Decrypt)?;
    let plaintext = cipher
        .decrypt(
            &nonce,
            Payload {
                msg: ciphertext,
                aad: &data[..HEADER_LEN],
            },
        )
        .map_err(|_| CryptoError::Decrypt)?;
    Ok(Opened {
        plaintext: Zeroizing::new(plaintext),
        params: header.params,
    })
}
