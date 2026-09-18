//! API Key 存取：通过系统凭据管理器读写百炼密钥。
//! 运行期内以进程内缓存兜底，避免反复触碰钥匙串触发授权弹窗。

use std::sync::Mutex;

const SERVICE: &str = "com.yezhou.simpleless";
const ACCOUNT: &str = "dashscope-api-key";

static CACHE: Mutex<Option<String>> = Mutex::new(None);

pub fn set_api_key(key: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())?;
    entry.set_password(key).map_err(|e| e.to_string())?;
    // 回读校验必须绕过缓存直读钥匙串，防止凭据存储静默失败导致假就绪状态
    let read_back = entry.get_password().map_err(|e| e.to_string())?;
    if read_back != key {
        return Err("API Key 写入后无法读回，保存可能未生效，请重试或检查系统设置".into());
    }
    *CACHE.lock().unwrap() = Some(key.to_string());
    Ok(())
}

pub fn get_api_key() -> Result<String, String> {
    if let Some(key) = CACHE.lock().unwrap().clone() {
        return Ok(key);
    }
    let entry = keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())?;
    let key = entry.get_password().map_err(|e| e.to_string())?;
    // 读失败不缓存，下次调用仍会重试钥匙串
    *CACHE.lock().unwrap() = Some(key.clone());
    Ok(key)
}
