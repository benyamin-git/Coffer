use coffer::crypto::{self, CryptoError};
use coffer::format::{FormatError, HEADER_LEN, Header, KdfParams};

const FAST: KdfParams = KdfParams {
    m_cost: 8,
    t_cost: 1,
    p_cost: 1,
};

fn seal_fast(password: &[u8], plaintext: &[u8]) -> Vec<u8> {
    crypto::seal_with_params(password, plaintext, FAST).expect("seal")
}

#[test]
fn roundtrip() {
    let sealed = seal_fast(b"correct horse", b"attack at dawn");
    let opened = crypto::open(b"correct horse", &sealed).expect("open");
    assert_eq!(&opened.plaintext[..], b"attack at dawn");
}

#[test]
fn roundtrip_empty_plaintext() {
    let sealed = seal_fast(b"pw", b"");
    let opened = crypto::open(b"pw", &sealed).expect("open");
    assert!(opened.plaintext.is_empty());
}

#[test]
fn roundtrip_large_plaintext() {
    let data = vec![0xAB; 1024 * 1024];
    let sealed = seal_fast(b"pw", &data);
    let opened = crypto::open(b"pw", &sealed).expect("open");
    assert_eq!(&opened.plaintext[..], &data[..]);
}

#[test]
fn wrong_password_fails() {
    let sealed = seal_fast(b"right", b"secret");
    let err = crypto::open(b"wrong", &sealed).unwrap_err();
    assert!(matches!(err, CryptoError::Decrypt));
}

#[test]
fn default_params_roundtrip_and_are_stored_in_header() {
    let sealed = crypto::seal(b"pw", b"hello").expect("seal");
    let (header, _) = Header::parse(&sealed).expect("parse");
    assert_eq!(header.params, KdfParams::default());
    let opened = crypto::open(b"pw", &sealed).expect("open");
    assert_eq!(&opened.plaintext[..], b"hello");
    assert_eq!(opened.params, KdfParams::default());
}

#[test]
fn every_header_byte_is_authenticated() {
    let sealed = seal_fast(b"pw", b"payload");
    for i in 0..HEADER_LEN {
        let mut tampered = sealed.clone();
        tampered[i] ^= 0x01;
        let result = crypto::open(b"pw", &tampered);
        assert!(result.is_err(), "header byte {i} was not authenticated");
    }
}

#[test]
fn ciphertext_bits_are_authenticated() {
    let sealed = seal_fast(b"pw", b"payload");
    for i in HEADER_LEN..sealed.len() {
        let mut tampered = sealed.clone();
        tampered[i] ^= 0x80;
        assert!(
            crypto::open(b"pw", &tampered).is_err(),
            "ciphertext byte {i} was not authenticated"
        );
    }
}

#[test]
fn truncated_ciphertext_fails() {
    let sealed = seal_fast(b"pw", b"payload");
    assert!(crypto::open(b"pw", &sealed[..sealed.len() - 1]).is_err());
    assert!(crypto::open(b"pw", &sealed[..HEADER_LEN]).is_err());
}

#[test]
fn truncated_header_fails() {
    let sealed = seal_fast(b"pw", b"payload");
    let err = crypto::open(b"pw", &sealed[..HEADER_LEN - 1]).unwrap_err();
    assert!(matches!(err, CryptoError::Format(FormatError::Truncated)));
}

#[test]
fn bad_magic_fails() {
    let sealed = seal_fast(b"pw", b"payload");
    let mut tampered = sealed.clone();
    tampered[0] ^= 0xFF;
    let err = crypto::open(b"pw", &tampered).unwrap_err();
    assert!(matches!(err, CryptoError::Format(FormatError::BadMagic)));
}

#[test]
fn salt_and_nonce_are_unique_per_seal() {
    let a = seal_fast(b"pw", b"same");
    let b = seal_fast(b"pw", b"same");
    let (ha, _) = Header::parse(&a).expect("parse");
    let (hb, _) = Header::parse(&b).expect("parse");
    assert_ne!(ha.salt, hb.salt);
    assert_ne!(ha.nonce, hb.nonce);
    assert_ne!(a, b);
}

#[test]
fn absurd_kdf_params_are_rejected_before_derivation() {
    let mut header = Header {
        params: FAST,
        salt: [7u8; 32],
        nonce: [9u8; 24],
    };
    header.params.m_cost = u32::MAX;
    let bytes = header.encode();
    let err = crypto::open(b"pw", &bytes).unwrap_err();
    assert!(matches!(err, CryptoError::Format(FormatError::BadParams)));

    let mut header = Header {
        params: FAST,
        salt: [7u8; 32],
        nonce: [9u8; 24],
    };
    header.params.t_cost = 1000;
    let err = crypto::open(b"pw", &header.encode()).unwrap_err();
    assert!(matches!(err, CryptoError::Format(FormatError::BadParams)));
}

#[test]
fn header_encode_parse_roundtrip() {
    let header = Header {
        params: FAST,
        salt: [1u8; 32],
        nonce: [2u8; 24],
    };
    let encoded = header.encode();
    let (parsed, rest) = Header::parse(&encoded).expect("parse");
    assert_eq!(parsed, header);
    assert!(rest.is_empty());
}
