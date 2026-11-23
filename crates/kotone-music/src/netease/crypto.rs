//! 网易云接口的两种加密。常量与算法照 api-enhanced `util/crypto.js`
//! （https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced，MIT，Copyright (c) 2013-2022 Binaryify）
//! 和 ncmapi `src/crypto`（https://github.com/two-mountains/ncmapi-rs，MIT，Copyright (c) 2021 TMs）。
//!
//! - **weapi**：`params = b64(AES-128-CBC(b64(AES-128-CBC(json, 预置键)), 随机 16 位键))`，IV 固定；
//!   `encSecKey = hex(RSA 无填充(倒过来的随机键))`。RSA 只用公钥，等于一次模幂，用 num-bigint 算，
//!   不引入 `rsa` crate（它有 RUSTSEC-2023-0071，私钥解密的计时攻击，和这里无关但会让审计不过）。
//! - **eapi**：`AES-128-ECB(url + "-36cd479b6b5-" + json + "-36cd479b6b5-" + md5("nobody"+url+"use"+json+"md5forencrypt"))`，大写十六进制。

use aes::Aes128;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyInit, KeyIvInit};
use md5::{Digest, Md5};
use num_bigint::BigUint;

const IV: &[u8; 16] = b"0102030405060708";
const PRESET_KEY: &[u8; 16] = b"0CoJUm6Qyw8W8jud";
const EAPI_KEY: &[u8; 16] = b"e82ckenh8dichen8";
const BASE62: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
/// api-enhanced 里 PEM 公钥的模数（openssl 解出来的 1024 位），指数 65537。
const MODULUS_HEX: &str = "E0B509F6259DF8642DBC35662901477DF22677EC152B5FF68ACE615BB7B725152B3AB17A876AEA8A5AA76D2E417629EC4EE341F56135FCCF695280104E0312ECBDA92557C93870114AF6C9D05C4F7F0C3685B7A46BEE255932575CCE10B424D813CFE4875D3E82047B97DDEF52741D546B8E289DC6935B3ECE0462DB0A22B8E7";
const EXPONENT: u32 = 65_537;

/// weapi 的两个表单字段。
pub(crate) struct Weapi {
    pub params: String,
    pub enc_sec_key: String,
}

pub(crate) fn weapi(text: &str) -> Weapi {
    let key = crate::random::chars(BASE62, 16);
    weapi_with_key(text, key.as_bytes())
}

pub(crate) fn weapi_with_key(text: &str, key: &[u8]) -> Weapi {
    let first = STANDARD.encode(aes_cbc(text.as_bytes(), PRESET_KEY));
    let params = STANDARD.encode(aes_cbc(first.as_bytes(), key));
    let reversed: Vec<u8> = key.iter().rev().copied().collect();
    let modulus = BigUint::parse_bytes(MODULUS_HEX.as_bytes(), 16).expect("modulus");
    let encrypted = BigUint::from_bytes_be(&reversed).modpow(&BigUint::from(EXPONENT), &modulus);
    Weapi {
        params,
        enc_sec_key: format!("{:0>256}", encrypted.to_str_radix(16)),
    }
}

/// eapi 的 `params` 字段。`url` 是 `/api/…` 路径。
pub(crate) fn eapi(url: &str, text: &str) -> String {
    let digest = hex::encode(Md5::digest(
        format!("nobody{url}use{text}md5forencrypt").as_bytes(),
    ));
    let data = format!("{url}-36cd479b6b5-{text}-36cd479b6b5-{digest}");
    hex::encode_upper(aes_ecb(data.as_bytes()))
}

fn aes_cbc(plain: &[u8], key: &[u8]) -> Vec<u8> {
    cbc::Encryptor::<Aes128>::new_from_slices(key, IV)
        .expect("16-byte key and IV")
        .encrypt_padded_vec_mut::<Pkcs7>(plain)
}

fn aes_ecb(plain: &[u8]) -> Vec<u8> {
    Aes128::new_from_slice(EAPI_KEY)
        .expect("16-byte key")
        .encrypt_padded_vec_mut::<Pkcs7>(plain)
}

#[cfg(test)]
pub(crate) fn eapi_decrypt(params_hex: &str) -> String {
    use cipher::BlockDecryptMut;
    let bytes = hex::decode(params_hex).unwrap();
    let plain = Aes128::new_from_slice(EAPI_KEY)
        .unwrap()
        .decrypt_padded_vec_mut::<Pkcs7>(&bytes)
        .unwrap();
    String::from_utf8(plain).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 期望值用 Node 的 crypto 按 api-enhanced 的算法算出来（2026-11-23）。
    #[test]
    fn eapi_matches_the_reference_implementation() {
        assert_eq!(
            eapi("/api/song/enhance/player/url", r#"{"ids":[1901371647],"br":320000}"#),
            "FA90B329E9614F79E79598F37DC2EDB430F8378D2A2796338F0BFDEAEF824A22975CDA9D96D79E6DC4A59218CDB8199F34A7690F11194897BE7CAFED11707561E6D4839DAD132A32AB99A4007D085096B8D5E18CFF3844E8B2BE2C8B228A372D628EF68FD21ADD179CFD149C65FA22BB9BE556E4ADB8E78516A5BA63BEF6ED12"
        );
        assert_eq!(
            eapi("/api/cloudsearch/pc", r#"{"s":"晴天","type":1}"#),
            "2B5D64177AA6460FBAA3DCB1285E28954BBB4F7556E09B0FB25750F12398BB5037682BEE8E4AD93F62BEEEE05F00B9E0BEE316873AA94B1A16E26ABB4C447A02DC2107351661EF0FFA09E7D09BE3900D226FEDBB50B3826BD67BD7C98D10FF56C6654F25B32E530C5B1AA5DFFEAC5C53"
        );
        let plain = eapi_decrypt(&eapi("/api/x", r#"{"a":1}"#));
        assert!(plain.starts_with(r#"/api/x-36cd479b6b5-{"a":1}-36cd479b6b5-"#));
    }

    #[test]
    fn weapi_matches_the_reference_implementation() {
        let form = weapi_with_key(
            r#"{"s":"晴天","type":1,"limit":2,"offset":0,"csrf_token":""}"#,
            b"abcdefghijklmnop",
        );
        assert_eq!(form.params, "q2U/Kw844bAZSbX4fA3vMcdq8pDCTLqUBXYwmEj9g1V/knZwAE6m5sJHnm/iEn9uItHxT1TP+bh37jdDG3InDbr/tXs21z8oBzz8RgQBZM/5g+aVAR+HoTDCqWDzHXPq");
        assert_eq!(form.enc_sec_key, "d15a1683c992095d0c234c19966605c5c5964911268bbeda8cb8d08d834913e59d53b32358903a121b5fca784c1f5ae44951fd02524df58ecc98e52cc7cf8689b42c2e93ddf05b0592512d87f5960467e2f086c018849d76014d323500e30f13ef4cafbb0cf5a66731a3f1776c75ca35d0062dac70a3e33245afabcf47938487");
        let random = weapi("{}");
        assert_eq!(random.enc_sec_key.len(), 256);
        assert!(!random.params.is_empty());
    }
}
