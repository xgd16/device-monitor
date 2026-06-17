//! Mihomo / Clash Meta 外部控制器状态采集。
//!
//! 默认读取 `http://192.168.1.110:9090`，可通过 `MIHOMO_CONTROLLER`
//! 覆盖。仅依赖标准库，避免给 5 秒采集循环引入额外异步运行时复杂度。

use super::MihomoInfo;
use serde_json::Value;
use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

const DEFAULT_CONTROLLER: &str = "http://192.168.1.110:9090";

pub fn collect() -> MihomoInfo {
    let controller = std::env::var("MIHOMO_CONTROLLER")
        .unwrap_or_else(|_| DEFAULT_CONTROLLER.to_string())
        .trim_end_matches('/')
        .to_string();

    match collect_inner(&controller) {
        Ok(mut info) => {
            info.available = true;
            info.controller = controller;
            info
        }
        Err(e) => MihomoInfo {
            available: false,
            controller,
            error: e,
            ..Default::default()
        },
    }
}

fn collect_inner(controller: &str) -> Result<MihomoInfo, String> {
    let configs = get_json(controller, "/configs")?;
    let proxies = get_json(controller, "/proxies")?;
    let connections = get_json(controller, "/connections")?;
    let version = get_json(controller, "/version").unwrap_or(Value::Null);

    let proxy_map = proxies
        .get("proxies")
        .and_then(Value::as_object)
        .ok_or_else(|| "mihomo /proxies missing proxies".to_string())?;

    let (active_group, active_proxy, proxy_chain) = resolve_active_proxy(proxy_map);

    Ok(MihomoInfo {
        available: true,
        controller: controller.to_string(),
        version: str_field(&version, "version"),
        mode: str_field(&configs, "mode"),
        tun_enabled: configs
            .get("tun")
            .and_then(|tun| tun.get("enable"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        active_group,
        active_proxy,
        proxy_chain,
        connection_count: connections
            .get("connections")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0),
        upload_total: connections
            .get("uploadTotal")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        download_total: connections
            .get("downloadTotal")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        error: String::new(),
    })
}

fn resolve_active_proxy(proxy_map: &serde_json::Map<String, Value>) -> (String, String, Vec<String>) {
    let start = ["GLOBAL", "Proxy", "🚀 节点选择", "节点选择"]
        .iter()
        .find(|name| proxy_map.contains_key(**name))
        .map(|name| (*name).to_string())
        .or_else(|| proxy_map.keys().find(|name| is_selector(proxy_map.get(*name))).cloned())
        .unwrap_or_default();

    if start.is_empty() {
        return (String::new(), String::new(), Vec::new());
    }

    let mut group = start.clone();
    let mut current = start;
    let mut chain = Vec::new();
    let mut seen = HashSet::new();

    loop {
        if !seen.insert(current.clone()) {
            break;
        }
        chain.push(current.clone());

        let Some(proxy) = proxy_map.get(&current) else {
            break;
        };
        let Some(now) = proxy.get("now").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
            break;
        };

        group = current;
        current = now.to_string();
    }

    let active_proxy = chain.last().cloned().unwrap_or_default();
    (group, active_proxy, chain)
}

fn is_selector(proxy: Option<&Value>) -> bool {
    proxy
        .and_then(|p| p.get("all"))
        .and_then(Value::as_array)
        .is_some()
}

fn str_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn get_json(controller: &str, path: &str) -> Result<Value, String> {
    let (host, port, base_path) = parse_http_url(controller)?;
    let full_path = if base_path == "/" {
        path.to_string()
    } else {
        format!("{}{}", base_path, path)
    };
    let mut stream = TcpStream::connect((host.as_str(), port))
        .map_err(|e| format!("connect mihomo {}:{} failed: {}", host, port, e))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(900)))
        .map_err(|e| format!("set read timeout failed: {}", e))?;
    stream
        .set_write_timeout(Some(Duration::from_millis(900)))
        .map_err(|e| format!("set write timeout failed: {}", e))?;

    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        full_path, host
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("request mihomo {} failed: {}", path, e))?;

    let mut resp = String::new();
    stream
        .read_to_string(&mut resp)
        .map_err(|e| format!("read mihomo {} failed: {}", path, e))?;

    let (head, body) = resp
        .split_once("\r\n\r\n")
        .ok_or_else(|| format!("invalid mihomo response for {}", path))?;
    if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
        return Err(format!("mihomo {} returned {}", path, head.lines().next().unwrap_or("")));
    }

    serde_json::from_str(body).map_err(|e| format!("parse mihomo {} failed: {}", path, e))
}

fn parse_http_url(url: &str) -> Result<(String, u16, String), String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| "MIHOMO_CONTROLLER only supports http:// URLs".to_string())?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = if let Some((host, port)) = authority.split_once(':') {
        let port = port
            .parse::<u16>()
            .map_err(|_| format!("invalid mihomo port: {}", port))?;
        (host.to_string(), port)
    } else {
        (authority.to_string(), 80)
    };
    if host.is_empty() {
        return Err("mihomo host is empty".to_string());
    }
    Ok((host, port, format!("/{}", path.trim_matches('/'))))
}
