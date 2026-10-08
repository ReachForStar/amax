//! 凭据加密模块
//!
//! Windows 用 DPAPI，密文绑定当前用户与机器；Linux 用 AES-256-GCM，密钥文件
//! （仅当前用户可读）随应用数据目录存放。两者都只能在本机解密，加密结果以 hex
//! 编码存入 SQLite，密文前缀由 db.rs 按平台区分。
//!
//! `key_dir` 是密钥文件所在目录（应用数据目录，Linux 侧使用；Windows 侧忽略）。

use std::path::Path;

#[cfg(windows)]
use std::ptr;
#[cfg(windows)]
use windows_sys::Win32::Foundation::LocalFree;
#[cfg(windows)]
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CryptProtectData, CryptUnprotectData,
};

/// DPAPI 加密 → hex 字符串
#[cfg(windows)]
pub fn encrypt(_key_dir: &Path, plaintext: &[u8]) -> Result<String, String> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(plaintext.len()).map_err(|_| "凭据长度超出 DPAPI 限制")?,
        pbData: plaintext.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };

    let ok = unsafe {
        CryptProtectData(
            &input,
            windows_sys::w!("AMAX Dashboard Secret"),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            0,
            &mut output,
        )
    };
    if ok == 0 {
        return Err("DPAPI 加密失败".into());
    }

    let encrypted = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) };
    let encoded = hex_encode(encrypted);
    unsafe { LocalFree(output.pbData as *mut std::ffi::c_void) };
    Ok(encoded)
}

/// DPAPI 解密 (输入 hex 字符串) → 原始字节
#[cfg(windows)]
pub fn decrypt(_key_dir: &Path, hex: &str) -> Result<Vec<u8>, String> {
    let encrypted = hex_decode(hex)?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(encrypted.len()).map_err(|_| "加密数据长度超出 DPAPI 限制")?,
        pbData: encrypted.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };

    let ok = unsafe {
        CryptUnprotectData(
            &input,
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            0,
            &mut output,
        )
    };
    if ok == 0 {
        return Err("DPAPI 解密失败 (可能是不同用户或机器)".into());
    }

    let decrypted =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe { LocalFree(output.pbData as *mut std::ffi::c_void) };
    Ok(decrypted)
}

#[cfg(unix)]
use ring::{
    aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey},
    rand::{SecureRandom, SystemRandom},
};
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

/// 密钥文件名：与 SQLite 库同目录，0600 权限
#[cfg(unix)]
const KEY_FILE_NAME: &str = "secret.key";
#[cfg(unix)]
const KEY_LEN: usize = 32;

/// AES-256-GCM 加密 → hex（nonce 前置）字符串
#[cfg(unix)]
pub fn encrypt(key_dir: &Path, plaintext: &[u8]) -> Result<String, String> {
    let key = load_or_create_key(key_dir)?;
    let sealing = LessSafeKey::new(aead_key(&key)?);

    let mut nonce_bytes = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce_bytes)
        .map_err(|_| "生成随机 nonce 失败".to_string())?;

    let mut in_out = plaintext.to_vec();
    sealing
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce_bytes),
            Aad::empty(),
            &mut in_out,
        )
        .map_err(|_| "AES-GCM 加密失败".to_string())?;

    let mut output = Vec::with_capacity(NONCE_LEN + in_out.len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&in_out);
    Ok(hex_encode(&output))
}

/// AES-256-GCM 解密 (输入 hex 字符串) → 原始字节
#[cfg(unix)]
pub fn decrypt(key_dir: &Path, hex: &str) -> Result<Vec<u8>, String> {
    let key = load_or_create_key(key_dir)?;
    let data = hex_decode(hex)?;
    if data.len() < NONCE_LEN + AES_256_GCM.tag_len() {
        return Err("密文长度不足".into());
    }

    let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
    let opening = LessSafeKey::new(aead_key(&key)?);
    let nonce =
        Nonce::try_assume_unique_for_key(nonce_bytes).map_err(|_| "nonce 长度非法".to_string())?;

    let mut in_out = ciphertext.to_vec();
    let plaintext = opening
        .open_in_place(nonce, Aad::empty(), &mut in_out)
        .map_err(|_| "AES-GCM 解密失败（密钥不匹配或数据损坏）".to_string())?;
    Ok(plaintext.to_vec())
}

#[cfg(unix)]
fn aead_key(key: &[u8; KEY_LEN]) -> Result<UnboundKey, String> {
    UnboundKey::new(&AES_256_GCM, key).map_err(|_| "初始化 AES-256-GCM 失败".to_string())
}

/// 首次调用生成密钥并以 0600 落盘，之后读取复用。
/// 解密路径也走这里：密钥文件意外丢失时按「解不开」上报，同时补一把新密钥供后续写入。
#[cfg(unix)]
fn load_or_create_key(dir: &Path) -> Result<[u8; KEY_LEN], String> {
    let path = dir.join(KEY_FILE_NAME);
    match std::fs::read(&path) {
        Ok(bytes) => key_from_bytes(&bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => create_key(&path),
        Err(error) => Err(format!("读取密钥文件失败: {error}")),
    }
}

#[cfg(unix)]
fn create_key(path: &Path) -> Result<[u8; KEY_LEN], String> {
    let mut key = [0u8; KEY_LEN];
    SystemRandom::new()
        .fill(&mut key)
        .map_err(|_| "生成随机密钥失败".to_string())?;

    let opened = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path);
    match opened {
        Ok(mut file) => {
            file.write_all(&key)
                .map_err(|error| format!("写入密钥文件失败: {error}"))?;
            Ok(key)
        }
        // 并发调用时先到者已建好，直接采用已有密钥；单实例插件保证跨进程不会同时写
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let bytes =
                std::fs::read(path).map_err(|error| format!("读取密钥文件失败: {error}"))?;
            key_from_bytes(&bytes)
        }
        Err(error) => Err(format!("创建密钥文件失败: {error}")),
    }
}

#[cfg(unix)]
fn key_from_bytes(bytes: &[u8]) -> Result<[u8; KEY_LEN], String> {
    bytes
        .try_into()
        .map_err(|_| "密钥文件损坏（长度不是 32 字节）".to_string())
}

/// 其他平台不提供不安全的明文降级。
#[cfg(not(any(windows, unix)))]
pub fn encrypt(_key_dir: &Path, _plaintext: &[u8]) -> Result<String, String> {
    Err("当前平台不支持安全存储".into())
}

#[cfg(not(any(windows, unix)))]
pub fn decrypt(_key_dir: &Path, _hex: &str) -> Result<Vec<u8>, String> {
    Err("当前平台不支持安全存储".into())
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("写入 String 不会失败");
    }
    encoded
}

fn hex_decode(value: &str) -> Result<Vec<u8>, String> {
    // 奇数长度即非法；长度校验与切分成对由 as_chunks 的余片一次性表达，不再分开判
    let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
    if !remainder.is_empty() {
        return Err("无效的 hex 字符串".into());
    }

    pairs
        .iter()
        .map(|pair| {
            let high = decode_nibble(pair[0])?;
            let low = decode_nibble(pair[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn decode_nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("hex 解码失败: 包含非法字符".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{hex_decode, hex_encode};

    #[test]
    fn hex_round_trip() {
        let value = b"session=abc123";
        assert_eq!(hex_decode(&hex_encode(value)).unwrap(), value);
    }

    #[test]
    fn invalid_hex_returns_error_without_panicking() {
        assert!(hex_decode("abc").is_err());
        assert!(hex_decode("你好").is_err());
        assert!(hex_decode("zz").is_err());
    }
}

#[cfg(all(test, unix))]
mod unix_tests {
    use super::{decrypt, encrypt};
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("amax-crypto-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("创建测试目录");
        dir
    }

    #[test]
    fn aes_round_trip_with_unicode() {
        let dir = test_dir("roundtrip");
        let value = "session=abc123; 中文凭据".as_bytes();
        let encrypted = encrypt(&dir, value).expect("加密应成功");
        assert_eq!(decrypt(&dir, &encrypted).expect("解密应成功"), value);
    }

    #[test]
    fn aes_nonce_makes_ciphertext_unique() {
        let dir = test_dir("nonce");
        let value = b"same plaintext";
        let first = encrypt(&dir, value).expect("加密应成功");
        let second = encrypt(&dir, value).expect("加密应成功");
        assert_ne!(first, second, "相同明文两次加密应因随机 nonce 而不同");
    }

    #[test]
    fn aes_decrypt_fails_with_another_key() {
        let plain_dir = test_dir("key-a");
        let other_dir = test_dir("key-b");
        let encrypted = encrypt(&plain_dir, b"secret").expect("加密应成功");
        assert!(decrypt(&other_dir, &encrypted).is_err());
    }

    #[test]
    fn aes_decrypt_fails_on_tampered_ciphertext() {
        let dir = test_dir("tamper");
        let encrypted = encrypt(&dir, b"secret").expect("加密应成功");
        let mut tampered = encrypted.clone();
        let last = tampered.pop().unwrap();
        tampered.push(if last == '0' { '1' } else { '0' });
        assert!(decrypt(&dir, &tampered).is_err());
    }

    #[test]
    fn key_file_is_owner_only() {
        let dir = test_dir("mode");
        let path = dir.join("secret.key");
        let _ = std::fs::remove_file(&path);
        encrypt(&dir, b"secret").expect("加密应成功");
        let mode = std::fs::metadata(&path)
            .expect("密钥文件应存在")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "密钥文件权限应为 0600");
    }
}
