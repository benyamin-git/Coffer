use thiserror::Error;

pub const MAGIC: [u8; 8] = *b"COFFERv1";
pub const KDF_ARGON2ID: u8 = 1;
pub const SALT_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;
pub const HEADER_LEN: usize = 8 + 1 + 4 + 4 + 4 + SALT_LEN + NONCE_LEN;

pub const DEFAULT_M_COST: u32 = 65_536;
pub const DEFAULT_T_COST: u32 = 3;
pub const DEFAULT_P_COST: u32 = 1;

pub const MIN_M_COST: u32 = 8;
pub const MAX_M_COST: u32 = 1_048_576;
pub const MAX_T_COST: u32 = 16;
pub const MAX_P_COST: u32 = 16;

#[derive(Debug, Error)]
pub enum FormatError {
    #[error("not a coffer vault (bad magic)")]
    BadMagic,
    #[error("unsupported vault format")]
    Unsupported,
    #[error("vault header has out-of-range KDF parameters")]
    BadParams,
    #[error("vault file is truncated")]
    Truncated,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct KdfParams {
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            m_cost: DEFAULT_M_COST,
            t_cost: DEFAULT_T_COST,
            p_cost: DEFAULT_P_COST,
        }
    }
}

impl KdfParams {
    fn is_sane(&self) -> bool {
        self.m_cost >= MIN_M_COST
            && self.m_cost <= MAX_M_COST
            && self.t_cost >= 1
            && self.t_cost <= MAX_T_COST
            && self.p_cost >= 1
            && self.p_cost <= MAX_P_COST
            && self.m_cost.checked_mul(1024).is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub params: KdfParams,
    pub salt: [u8; SALT_LEN],
    pub nonce: [u8; NONCE_LEN],
}

impl Header {
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[0..8].copy_from_slice(&MAGIC);
        out[8] = KDF_ARGON2ID;
        out[9..13].copy_from_slice(&self.params.m_cost.to_le_bytes());
        out[13..17].copy_from_slice(&self.params.t_cost.to_le_bytes());
        out[17..21].copy_from_slice(&self.params.p_cost.to_le_bytes());
        out[21..53].copy_from_slice(&self.salt);
        out[53..77].copy_from_slice(&self.nonce);
        out
    }

    pub fn parse(data: &[u8]) -> Result<(Self, &[u8]), FormatError> {
        if data.len() < HEADER_LEN {
            return Err(FormatError::Truncated);
        }
        if data[0..8] != MAGIC {
            return Err(FormatError::BadMagic);
        }
        if data[8] != KDF_ARGON2ID {
            return Err(FormatError::Unsupported);
        }
        let params = KdfParams {
            m_cost: u32::from_le_bytes(data[9..13].try_into().expect("4 bytes")),
            t_cost: u32::from_le_bytes(data[13..17].try_into().expect("4 bytes")),
            p_cost: u32::from_le_bytes(data[17..21].try_into().expect("4 bytes")),
        };
        if !params.is_sane() {
            return Err(FormatError::BadParams);
        }
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&data[21..53]);
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&data[53..77]);
        Ok((
            Self {
                params,
                salt,
                nonce,
            },
            &data[HEADER_LEN..],
        ))
    }
}
