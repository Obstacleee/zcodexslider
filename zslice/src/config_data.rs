//! `ConfigurationData`: the slicer configuration Z-Suite embeds in every `.zcodex2`.
//!
//! Format (little-endian), as written by Z-Suite 3.7:
//! `u32 total | u32 n1 | RSA-OAEP(AES key) | u32 n2 | RSA-OAEP(IV) | u32 n3 | AES-256-CBC(JSON)`
//! with a fresh random key and IV per file, encrypted with Zortrax's RSA-2048 public key. Only Zortrax can
//! decrypt it; Z-Suite itself just keeps the bytes when opening a file, but it **refuses to open a file
//! that has no `ConfigurationData` entry**, so we always write one.

use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use rand::RngCore;
use rsa::{BigUint, Oaep, RsaPublicKey};
use serde_json::{json, Value};
use sha1::Sha1;

use crate::settings::Settings;

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;

/// Zortrax's public key (from Z-SUITE.dll): exponent and modulus, base64.
const EXPONENT_B64: &str = "AQAB";
const MODULUS_B64: &str = "qCwJL1EZ/XO+tfzIu1cPniwuq5SzejwxZrztAlAFqA587GiMqHKHZoijiNCnXsGsZQadVYIPrgqF125f4Rao74ibmO9b0mh68123HAnWS6R/WAPo+VYcakdswZ8tL8ks3JSC4OJ14egWretGhMREc+hhY3jPAqK7Hed60ExvNN/HGACgBJM9ItFIp2BwJSwpBDvqz05JMbCDOpi/gj7B/cPuhCKS92tl/2oDdn7AQ2aTDoWya9Az01P/2ZKjRDWU+usmv7T2NeqoHmogfBfuoL7rVerSD4QXokmKXYlb1m2/JNN3G4FRm1arRkRwiYrh/J/ugzMIXEifaTaUY0B34Q==";

/// All public fields of `Zortrax.Configuration.Configuration` with default values (0 / false / "").
const DEFAULTS: &str = include_str!("../templates/configuration_defaults.json");

fn b64_decode(s: &str) -> Vec<u8> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        if let Some(v) = val(c) {
            acc = (acc << 6) | v as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
                acc &= (1 << bits) - 1;
            }
        }
    }
    out
}

pub fn zortrax_public_key() -> RsaPublicKey {
    let e = BigUint::from_bytes_be(&b64_decode(EXPONENT_B64));
    let n = BigUint::from_bytes_be(&b64_decode(MODULUS_B64));
    RsaPublicKey::new(n, e).expect("embedded public key is valid")
}

/// JSON of the configuration object, filled with what we know from the settings.
pub fn configuration_json(s: &Settings) -> Vec<u8> {
    let mut v: Value = serde_json::from_str(DEFAULTS).expect("defaults template");
    let set = |v: &mut Value, k: &str, x: Value| {
        if v.get(k).is_some() {
            v[k] = x;
        }
    };
    set(&mut v, "SliceThickness", json!(s.layer_height));
    set(&mut v, "NozzleDiameter", json!(s.nozzle_diameter));
    set(&mut v, "TempExtruder", json!(s.nozzle_temp));
    set(&mut v, "TempBed", json!(s.bed_temp));
    set(&mut v, "TempChamber", json!(-1));
    set(&mut v, "SeamType", json!(0));
    set(&mut v, "AvoidOverhangs", json!(false));
    set(&mut v, "SUPPORT_LITE", json!(false));
    set(&mut v, "SB_MinWidth", json!(s.support_width));
    set(&mut v, "SB_HorizontalSpace", json!(s.support_xy_gap));
    set(&mut v, "SupportBottomNumber", json!(s.support_bottom_layers));
    serde_json::to_vec(&v).expect("json")
}

/// Hybrid encryption exactly like Z-Suite: random AES-256 key and IV, both RSA-OAEP(SHA-1) encrypted.
pub fn encrypt_with(key: &RsaPublicKey, json: &[u8]) -> Result<Vec<u8>, String> {
    let mut rng = rand::rngs::OsRng;
    let mut aes_key = [0u8; 32];
    let mut iv = [0u8; 16];
    rng.fill_bytes(&mut aes_key);
    rng.fill_bytes(&mut iv);
    let cipher = Aes256CbcEnc::new(&aes_key.into(), &iv.into()).encrypt_padded_vec_mut::<Pkcs7>(json);
    let enc_key = key.encrypt(&mut rng, Oaep::new::<Sha1>(), &aes_key).map_err(|e| format!("RSA : {e}"))?;
    let enc_iv = key.encrypt(&mut rng, Oaep::new::<Sha1>(), &iv).map_err(|e| format!("RSA : {e}"))?;

    let mut body = Vec::new();
    for part in [&enc_key[..], &enc_iv[..], &cipher[..]] {
        body.extend((part.len() as u32).to_le_bytes());
        body.extend_from_slice(part);
    }
    let mut out = Vec::with_capacity(body.len() + 4);
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    Ok(out)
}

/// The `ConfigurationData` entry for a file sliced with `s`.
pub fn make_configuration_data(s: &Settings) -> Result<Vec<u8>, String> {
    encrypt_with(&zortrax_public_key(), &configuration_json(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::{BlockDecryptMut, KeyIvInit};
    use rsa::RsaPrivateKey;

    type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

    fn split(data: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let total = u32::from_le_bytes(data[..4].try_into().unwrap()) as usize;
        assert_eq!(total, data.len() - 4);
        let mut p = 4;
        let mut parts = Vec::new();
        for _ in 0..3 {
            let n = u32::from_le_bytes(data[p..p + 4].try_into().unwrap()) as usize;
            parts.push(data[p + 4..p + 4 + n].to_vec());
            p += 4 + n;
        }
        assert_eq!(p, data.len());
        (parts[0].clone(), parts[1].clone(), parts[2].clone())
    }

    #[test]
    fn embedded_key_is_rsa_2048_with_exponent_65537() {
        use rsa::traits::PublicKeyParts;
        let k = zortrax_public_key();
        assert_eq!(k.n().bits(), 2048);
        assert_eq!(k.e(), &BigUint::from(65537u32));
    }

    #[test]
    fn roundtrip_with_a_test_key() {
        let mut rng = rand::rngs::OsRng;
        let private = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public = RsaPublicKey::from(&private);
        let json = configuration_json(&Settings::default());
        let blob = encrypt_with(&public, &json).unwrap();
        let (ek, ei, ct) = split(&blob);
        assert_eq!((ek.len(), ei.len()), (256, 256));
        let key = private.decrypt(Oaep::new::<Sha1>(), &ek).unwrap();
        let iv = private.decrypt(Oaep::new::<Sha1>(), &ei).unwrap();
        let plain = Aes256CbcDec::new(key.as_slice().into(), iv.as_slice().into()).decrypt_padded_vec_mut::<Pkcs7>(&ct).unwrap();
        assert_eq!(plain, json);
        let v: Value = serde_json::from_slice(&plain).unwrap();
        assert_eq!(v["TempExtruder"], 210);
        assert_eq!(v["TempBed"], 30);
        assert!(v.as_object().unwrap().len() > 300);
    }

    #[test]
    fn layout_matches_a_real_z_suite_file() {
        // Z-Suite's own ConfigurationData has the same block structure (two 256-byte RSA blocks)
        let Ok(f) = zcodex2::Zcodex2::open(r"C:\Users\Lucas Delon\Downloads\sentinel 2.zcodex2") else { return };
        let real = f.entry(zcodex2::CONFIGURATION_DATA).unwrap();
        let (ek, ei, ct) = split(real);
        assert_eq!((ek.len(), ei.len()), (256, 256));
        assert_eq!(ct.len() % 16, 0);
        let mine = make_configuration_data(&Settings::default()).unwrap();
        let (mk, mi, mc) = split(&mine);
        assert_eq!((mk.len(), mi.len()), (256, 256));
        assert_eq!(mc.len() % 16, 0);
    }
}
