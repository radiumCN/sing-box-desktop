use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose};
use serde_json::{Value, json};
use serde_yaml::Value as YamlValue;
use url::Url;
use crate::types::{ProxyNode, SubType};

/// Detect subscription type from content or URL
pub fn detect_sub_type(content: &str, url: &str) -> SubType {
    let content_trimmed = content.trim();
    // Clash YAML has 'proxies:' key
    if content_trimmed.contains("proxies:") || content_trimmed.contains("proxy-groups:") {
        return SubType::Clash;
    }
    // V2Ray base64 encoded
    if let Ok(decoded) = general_purpose::STANDARD.decode(content_trimmed.as_bytes()) {
        if let Ok(text) = String::from_utf8(decoded) {
            if text.contains("vmess://")
                || text.contains("vless://")
                || text.contains("ss://")
                || text.contains("trojan://")
                || text.contains("hysteria2://")
                || text.contains("hy2://")
                || text.contains("tuic://")
                || text.contains("anytls://")
            {
                return SubType::V2ray;
            }
        }
    }
    // Single node links
    if content_trimmed.starts_with("vmess://")
        || content_trimmed.starts_with("vless://")
        || content_trimmed.starts_with("ss://")
        || content_trimmed.starts_with("trojan://")
        || content_trimmed.starts_with("hysteria2://")
        || content_trimmed.starts_with("hy2://")
        || content_trimmed.starts_with("tuic://")
        || content_trimmed.starts_with("anytls://")
    {
        return SubType::V2ray;
    }
    // SIP008 JSON
    if let Ok(v) = serde_json::from_str::<Value>(content_trimmed) {
        if v.get("servers").is_some() && v.get("version").is_some() {
            return SubType::Sip008;
        }
    }
    let _ = url;
    SubType::Unknown
}

/// Parse subscription content into ProxyNode list + sing-box outbounds JSON
pub fn parse_subscription(
    content: &str,
    sub_id: &str,
) -> Result<(Vec<ProxyNode>, Vec<Value>)> {
    let sub_type = detect_sub_type(content, "");
    match sub_type {
        SubType::Clash => parse_clash(content, sub_id),
        SubType::V2ray => parse_v2ray(content, sub_id),
        SubType::Sip008 => parse_sip008(content, sub_id),
        SubType::Unknown => Err(anyhow!("无法识别的订阅格式")),
    }
}

fn parse_clash(content: &str, sub_id: &str) -> Result<(Vec<ProxyNode>, Vec<Value>)> {
    let yaml: YamlValue = serde_yaml::from_str(content)
        .map_err(|e| anyhow!("Clash YAML 解析失败: {}", e))?;

    let proxies = yaml["proxies"]
        .as_sequence()
        .ok_or_else(|| anyhow!("未找到 proxies 字段"))?;

    let mut nodes = Vec::new();
    let mut outbounds = Vec::new();

    for proxy in proxies {
        let name = proxy["name"].as_str().unwrap_or("Unknown").to_string();
        let proto = proxy["type"].as_str().unwrap_or("unknown").to_string();
        let server = proxy["server"].as_str().unwrap_or("").to_string();
        let port = proxy["port"].as_u64().unwrap_or(0) as u16;

        let node = ProxyNode {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.clone(),
            group: "默认".to_string(),
            protocol: proto.clone(),
            server: server.clone(),
            port,
            latency: None,
            download_speed: None,
            is_active: false,
            subscription_id: Some(sub_id.to_string()),
        };
        nodes.push(node);

        if let Some(outbound) = clash_yaml_proxy_to_singbox(proxy, &name) {
            outbounds.push(outbound);
        }
    }

    Ok((nodes, outbounds))
}

/// Build a sing-box transport object from a Clash YAML proxy.
/// Returns None for plain TCP (transport field must be omitted entirely).
fn clash_transport(network: &str, proxy: &YamlValue) -> Option<Value> {
    match network {
        "tcp" | "" => None,
        "ws" => {
            let path = proxy["ws-opts"]["path"].as_str()
                .or_else(|| proxy["ws-path"].as_str())
                .unwrap_or("/");
            let host = proxy["ws-opts"]["headers"]["Host"].as_str()
                .or_else(|| proxy["ws-headers"]["Host"].as_str())
                .unwrap_or("");
            Some(json!({ "type": "ws", "path": path, "headers": { "Host": host } }))
        }
        "grpc" => {
            let svc = proxy["grpc-opts"]["grpc-service-name"].as_str().unwrap_or("");
            Some(json!({ "type": "grpc", "service_name": svc }))
        }
        "h2" | "http" => {
            let path = proxy["h2-opts"]["path"][0].as_str().unwrap_or("/");
            let host = proxy["h2-opts"]["host"][0].as_str().unwrap_or("");
            Some(json!({ "type": "http", "path": path, "host": [host] }))
        }
        "httpupgrade" => {
            let path = proxy["httpupgrade-opts"]["path"].as_str().unwrap_or("/");
            let host = proxy["httpupgrade-opts"]["host"].as_str().unwrap_or("");
            Some(json!({ "type": "httpupgrade", "path": path, "host": host }))
        }
        other => Some(json!({ "type": other })),
    }
}

/// Clash `skip-cert-verify` flag — default false (verify the certificate). TLS is
/// secure by default; only a node that explicitly opts out skips verification.
fn clash_skip_verify(proxy: &YamlValue) -> bool {
    proxy["skip-cert-verify"].as_bool().unwrap_or(false)
}

/// Read an explicit "skip TLS verification" flag from URI query params. Defaults to
/// false (verify) — only an explicit allowInsecure/insecure = 1/true opts out.
fn param_insecure(params: &std::collections::HashMap<String, String>) -> bool {
    params.get("allowInsecure")
        .or_else(|| params.get("allow_insecure"))
        .or_else(|| params.get("insecure"))
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false)
}

/// Convert a Clash `ss` proxy's structured `plugin` / `plugin-opts` into the sing-box
/// (SIP003) representation: a normalised plugin name plus a semicolon-separated
/// `plugin_opts` string. Returns `None` when no plugin is configured. Mirrors the
/// `ss://` single-link mapping in `parse_ss` so both import paths behave identically.
///
/// Clash carries plugin options as a structured map (`plugin-opts: { mode: tls, host: … }`)
/// whereas sing-box wants the raw SIP003 args string, so each known plugin is translated
/// explicitly; unknown plugins fall back to a best-effort `k=v;flag` serialisation rather
/// than being dropped.
fn clash_ss_plugin(proxy: &YamlValue) -> Option<(String, String)> {
    let plugin = proxy["plugin"].as_str().filter(|p| !p.is_empty())?;
    let opts = &proxy["plugin-opts"];

    // Read a scalar (string / integer / bool) plugin-opt as a string.
    let scalar = |key: &str| -> Option<String> {
        opts[key].as_str().map(|v| v.to_string())
            .or_else(|| opts[key].as_u64().map(|n| n.to_string()))
    };
    let flag = |key: &str| -> bool { opts[key].as_bool().unwrap_or(false) };

    match plugin {
        "obfs" | "simple-obfs" | "obfs-local" => {
            // Clash: mode = http|tls, host = <domain> → sing-box obfs=<mode>;obfs-host=<host>
            let mode = scalar("mode").unwrap_or_else(|| "http".to_string());
            let mut parts = vec![format!("obfs={}", mode)];
            if let Some(host) = scalar("host") {
                parts.push(format!("obfs-host={}", host));
            }
            Some(("obfs-local".to_string(), parts.join(";")))
        }
        "v2ray-plugin" => {
            let mode = scalar("mode").unwrap_or_else(|| "websocket".to_string());
            let mut parts = vec![format!("mode={}", mode)];
            if flag("tls") {
                parts.push("tls".to_string());
            }
            if let Some(host) = scalar("host") {
                parts.push(format!("host={}", host));
            }
            if let Some(path) = scalar("path") {
                parts.push(format!("path={}", path));
            }
            if flag("mux") {
                parts.push("mux".to_string());
            }
            Some(("v2ray-plugin".to_string(), parts.join(";")))
        }
        "shadow-tls" => {
            let mut parts: Vec<String> = Vec::new();
            if let Some(host) = scalar("host") { parts.push(format!("host={}", host)); }
            if let Some(pw) = scalar("password") { parts.push(format!("password={}", pw)); }
            if let Some(ver) = scalar("version") { parts.push(format!("version={}", ver)); }
            Some(("shadow-tls".to_string(), parts.join(";")))
        }
        other => {
            // Unknown plugin: serialise each scalar opt as `k=v` and each true bool as a
            // bare flag, so the node is preserved with its options rather than dropped.
            let mut parts: Vec<String> = Vec::new();
            if let Some(map) = opts.as_mapping() {
                for (k, v) in map {
                    if let Some(key) = k.as_str() {
                        if let Some(b) = v.as_bool() {
                            if b { parts.push(key.to_string()); }
                        } else if let Some(val) = v.as_str() {
                            parts.push(format!("{}={}", key, val));
                        } else if let Some(n) = v.as_u64() {
                            parts.push(format!("{}={}", key, n));
                        }
                    }
                }
            }
            Some((other.to_string(), parts.join(";")))
        }
    }
}

fn clash_yaml_proxy_to_singbox(proxy: &YamlValue, tag: &str) -> Option<Value> {
    let proto = proxy["type"].as_str()?;
    let server = proxy["server"].as_str()?;
    let port = proxy["port"].as_u64()?;

    match proto {
        "ss" => {
            let password = proxy["password"].as_str().unwrap_or("");
            let cipher = proxy["cipher"].as_str().unwrap_or("aes-128-gcm");
            let mut ob = json!({
                "type": "shadowsocks",
                "tag": tag,
                "server": server,
                "server_port": port,
                "method": cipher,
                "password": password
            });
            // SIP003 plugin (obfs / v2ray-plugin / shadow-tls …): translate Clash's
            // structured plugin-opts into sing-box's `plugin` + `plugin_opts` string.
            if let Some((plugin, opts)) = clash_ss_plugin(proxy) {
                ob["plugin"] = json!(plugin);
                if !opts.is_empty() {
                    ob["plugin_opts"] = json!(opts);
                }
            }
            Some(ob)
        }
        "vmess" => {
            let uuid = proxy["uuid"].as_str().unwrap_or("");
            let alter_id = proxy["alterId"].as_u64().unwrap_or(0);
            let network = proxy["network"].as_str().unwrap_or("tcp");
            let tls = proxy["tls"].as_bool().unwrap_or(false);
            let mut ob = json!({
                "type": "vmess",
                "tag": tag,
                "server": server,
                "server_port": port,
                "uuid": uuid,
                "alter_id": alter_id,
                "security": "auto"
            });
            if let Some(t) = clash_transport(network, proxy) {
                ob["transport"] = t;
            }
            if tls {
                ob["tls"] = json!({ "enabled": true, "insecure": clash_skip_verify(proxy) });
            }
            Some(ob)
        }
        "vless" => {
            let uuid = proxy["uuid"].as_str().unwrap_or("");
            let network = proxy["network"].as_str().unwrap_or("tcp");
            let flow = proxy["flow"].as_str().unwrap_or("");
            let sni = proxy["servername"].as_str()
                .or_else(|| proxy["sni"].as_str())
                .unwrap_or(server);
            let reality_opts = &proxy["reality-opts"];
            let has_reality = reality_opts.is_mapping();
            // Reality implies TLS even if the `tls` flag is absent.
            let tls = proxy["tls"].as_bool().unwrap_or(false) || has_reality;
            let mut ob = json!({
                "type": "vless",
                "tag": tag,
                "server": server,
                "server_port": port,
                "uuid": uuid,
                "flow": flow
            });
            if let Some(t) = clash_transport(network, proxy) {
                ob["transport"] = t;
            }
            if tls {
                let fp = proxy["client-fingerprint"].as_str().unwrap_or("chrome");
                let mut tls_obj = json!({
                    "enabled": true,
                    "server_name": sni,
                    "utls": { "enabled": true, "fingerprint": fp }
                });
                if has_reality {
                    // Reality verifies the server via its public key; never set `insecure`.
                    let pbk = reality_opts["public-key"].as_str().unwrap_or("");
                    let sid = reality_opts["short-id"].as_str().unwrap_or("");
                    tls_obj["reality"] = json!({
                        "enabled": true,
                        "public_key": pbk,
                        "short_id": sid
                    });
                } else {
                    tls_obj["insecure"] = json!(clash_skip_verify(proxy));
                }
                ob["tls"] = tls_obj;
            }
            Some(ob)
        }
        "trojan" => {
            let password = proxy["password"].as_str().unwrap_or("");
            let sni = proxy["sni"].as_str().unwrap_or(server);
            Some(json!({
                "type": "trojan",
                "tag": tag,
                "server": server,
                "server_port": port,
                "password": password,
                "tls": { "enabled": true, "server_name": sni, "insecure": clash_skip_verify(proxy) }
            }))
        }
        "hysteria2" | "hy2" => {
            let password = proxy["password"].as_str().unwrap_or("");
            let sni = proxy["sni"].as_str().unwrap_or(server);
            Some(json!({
                "type": "hysteria2",
                "tag": tag,
                "server": server,
                "server_port": port,
                "password": password,
                "tls": { "enabled": true, "server_name": sni, "insecure": clash_skip_verify(proxy) }
            }))
        }
        "tuic" => {
            let uuid = proxy["uuid"].as_str().unwrap_or("");
            let password = proxy["password"].as_str().unwrap_or("");
            let sni = proxy["sni"].as_str().unwrap_or(server);
            let congestion = proxy["congestion-controller"].as_str()
                .or_else(|| proxy["congestion_control"].as_str())
                .unwrap_or("bbr");
            let udp_relay_mode = proxy["udp-relay-mode"].as_str().unwrap_or("native");
            let mut tls = json!({ "enabled": true, "server_name": sni, "insecure": clash_skip_verify(proxy) });
            if let Some(alpn) = proxy["alpn"].as_sequence() {
                let list: Vec<&str> = alpn.iter().filter_map(|v| v.as_str()).collect();
                if !list.is_empty() {
                    tls["alpn"] = json!(list);
                }
            }
            Some(json!({
                "type": "tuic",
                "tag": tag,
                "server": server,
                "server_port": port,
                "uuid": uuid,
                "password": password,
                "congestion_control": congestion,
                "udp_relay_mode": udp_relay_mode,
                "tls": tls
            }))
        }
        "anytls" => {
            let password = proxy["password"].as_str().unwrap_or("");
            let sni = proxy["sni"].as_str().unwrap_or(server);
            Some(json!({
                "type": "anytls",
                "tag": tag,
                "server": server,
                "server_port": port,
                "password": password,
                "tls": { "enabled": true, "server_name": sni, "insecure": clash_skip_verify(proxy) }
            }))
        }
        _ => None,
    }
}

fn parse_v2ray(content: &str, sub_id: &str) -> Result<(Vec<ProxyNode>, Vec<Value>)> {
    let text = if let Ok(decoded) = general_purpose::STANDARD.decode(content.trim().as_bytes()) {
        String::from_utf8(decoded).unwrap_or_else(|_| content.to_string())
    } else if let Ok(decoded) = general_purpose::URL_SAFE.decode(content.trim().as_bytes()) {
        String::from_utf8(decoded).unwrap_or_else(|_| content.to_string())
    } else {
        content.to_string()
    };

    let mut nodes = Vec::new();
    let mut outbounds = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok((node, outbound)) = parse_node_link(line, sub_id) {
            outbounds.push(outbound);
            nodes.push(node);
        }
    }

    if nodes.is_empty() {
        return Err(anyhow!("未解析到有效节点"));
    }

    Ok((nodes, outbounds))
}

fn parse_node_link(link: &str, sub_id: &str) -> Result<(ProxyNode, Value)> {
    if link.starts_with("vmess://") {
        parse_vmess(link, sub_id)
    } else if link.starts_with("vless://") {
        parse_vless(link, sub_id)
    } else if link.starts_with("ss://") {
        parse_ss(link, sub_id)
    } else if link.starts_with("trojan://") {
        parse_trojan(link, sub_id)
    } else if link.starts_with("hysteria2://") || link.starts_with("hy2://") {
        parse_hysteria2(link, sub_id)
    } else if link.starts_with("tuic://") {
        parse_tuic(link, sub_id)
    } else if link.starts_with("anytls://") {
        parse_anytls(link, sub_id)
    } else {
        Err(anyhow!("不支持的链接类型: {}", link))
    }
}

fn parse_vmess(link: &str, sub_id: &str) -> Result<(ProxyNode, Value)> {
    let encoded = link.trim_start_matches("vmess://");
    let json_str = String::from_utf8(
        general_purpose::STANDARD.decode(encoded)
            .or_else(|_| general_purpose::URL_SAFE.decode(encoded))?
    )?;
    let v: Value = serde_json::from_str(&json_str)?;
    let name = v["ps"].as_str().or(v["add"].as_str()).unwrap_or("vmess").to_string();
    let server = v["add"].as_str().unwrap_or("").to_string();
    let port: u16 = v["port"].as_u64()
        .or_else(|| v["port"].as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(443) as u16;
    let uuid = v["id"].as_str().unwrap_or("").to_string();
    let alter_id = v["aid"].as_u64()
        .or_else(|| v["aid"].as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0);
    let network = v["net"].as_str().unwrap_or("tcp").to_string();
    let tls = v["tls"].as_str().map(|s| s == "tls").unwrap_or(false);
    let sni = v["sni"].as_str().or(v["host"].as_str()).unwrap_or("").to_string();

    let mut outbound = json!({
        "type": "vmess",
        "tag": name,
        "server": server,
        "server_port": port,
        "uuid": uuid,
        "alter_id": alter_id,
        "security": "auto"
    });
    // Only set transport when it's not plain TCP
    if network != "tcp" && !network.is_empty() {
        let path = v["path"].as_str().unwrap_or("/").to_string();
        let host = v["host"].as_str().unwrap_or(&sni).to_string();
        let transport = match network.as_str() {
            "ws" => json!({ "type": "ws", "path": path, "headers": { "Host": host } }),
            "grpc" => json!({ "type": "grpc", "service_name": path }),
            "h2" | "http" => json!({ "type": "http", "path": path, "host": [host] }),
            other => json!({ "type": other }),
        };
        outbound["transport"] = transport;
    }
    if tls {
        // Secure by default; vmess links may opt out via an explicit allowInsecure
        // field (carried as bool / "1"/"true" / 1 depending on the exporter).
        let insecure = v["allowInsecure"].as_bool()
            .or_else(|| v["allowInsecure"].as_str().map(|s| s == "1" || s == "true"))
            .or_else(|| v["allowInsecure"].as_u64().map(|n| n == 1))
            .unwrap_or(false);
        outbound["tls"] = json!({ "enabled": true, "server_name": sni, "insecure": insecure });
    }

    let node = ProxyNode {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.clone(),
        group: "默认".to_string(),
        protocol: "vmess".to_string(),
        server,
        port,
        latency: None,
        download_speed: None,
        is_active: false,
        subscription_id: Some(sub_id.to_string()),
    };

    Ok((node, outbound))
}

fn parse_vless(link: &str, sub_id: &str) -> Result<(ProxyNode, Value)> {
    let url = Url::parse(link)?;
    let uuid = url.username().to_string();
    let server = url.host_str().unwrap_or("").to_string();
    let port = url.port().unwrap_or(443);
    let params: std::collections::HashMap<String, String> = url.query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let name = url.fragment().unwrap_or(&server).to_string();
    let network = params.get("type").map(|s| s.as_str()).unwrap_or("tcp").to_string();
    let security = params.get("security").map(|s| s.as_str()).unwrap_or("none").to_string();
    let sni = params.get("sni").cloned().unwrap_or_else(|| server.clone());
    let flow = params.get("flow").cloned().unwrap_or_default();

    let mut outbound = json!({
        "type": "vless",
        "tag": name,
        "server": server,
        "server_port": port,
        "uuid": uuid,
        "flow": flow
    });
    // Only add transport for non-TCP networks
    if network != "tcp" && !network.is_empty() {
        let path = params.get("path").cloned().unwrap_or_else(|| "/".to_string());
        let host = params.get("host").cloned().unwrap_or_else(|| server.clone());
        let svc = params.get("serviceName").or(params.get("service_name")).cloned().unwrap_or_default();
        let transport = match network.as_str() {
            "ws" => json!({ "type": "ws", "path": path, "headers": { "Host": host } }),
            "grpc" => json!({ "type": "grpc", "service_name": svc }),
            "h2" | "http" => json!({ "type": "http", "path": path, "host": [host] }),
            "httpupgrade" => json!({ "type": "httpupgrade", "path": path, "host": host }),
            other => json!({ "type": other }),
        };
        outbound["transport"] = transport;
    }
    if security == "tls" || security == "reality" {
        let fp = params.get("fp").cloned().unwrap_or_else(|| "chrome".to_string());
        let mut tls = json!({
            "enabled": true,
            "server_name": sni,
            "utls": { "enabled": true, "fingerprint": fp }
        });
        if security == "reality" {
            // Reality performs its own certificate verification via the public key,
            // so `insecure` must NOT be set. Public key (pbk) and short id (sid)
            // are mandatory for the Reality handshake to succeed.
            let pbk = params.get("pbk").cloned().unwrap_or_default();
            let sid = params.get("sid").cloned().unwrap_or_default();
            tls["reality"] = json!({
                "enabled": true,
                "public_key": pbk,
                "short_id": sid
            });
        } else {
            // Plain TLS: verify by default; honor an explicit allowInsecure param.
            tls["insecure"] = json!(param_insecure(&params));
        }
        outbound["tls"] = tls;
    }

    let node = ProxyNode {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.clone(),
        group: "默认".to_string(),
        protocol: "vless".to_string(),
        server,
        port,
        latency: None,
        download_speed: None,
        is_active: false,
        subscription_id: Some(sub_id.to_string()),
    };

    Ok((node, outbound))
}

/// Resolve `(method, password)` from a Shadowsocks SIP002 userinfo.
///
/// The standard form encodes `base64(method:password)` in the userinfo; some links use
/// the plain `method:password` form instead. We read the RAW userinfo straight from the
/// link (the substring before the first `@`) rather than `url.username()`, because the
/// `url` crate follows WHATWG and percent-encodes base64 characters (`=` → `%3D`,
/// `/` → `%2F`), which corrupts the blob and breaks decoding. Both base64 alphabets
/// (url-safe and standard) and optional padding are tolerated.
fn ss_method_password(link: &str, url: &Url) -> (String, String) {
    let raw = link
        .trim_start_matches("ss://")
        .split('@')
        .next()
        .unwrap_or("");
    // Try base64(method:password). The presence of a ':' in the decoded text is the
    // signal that this really was a base64 userinfo (a plain `method:password` raw
    // contains a ':' which is not a valid base64 symbol, so decoding fails and we fall
    // through to the plain branch below).
    let trimmed = raw.trim_end_matches('=');
    let decoded = general_purpose::URL_SAFE_NO_PAD
        .decode(trimmed)
        .or_else(|_| general_purpose::STANDARD_NO_PAD.decode(trimmed))
        .ok()
        .and_then(|b| String::from_utf8(b).ok());
    if let Some(s) = decoded {
        if let Some((method, password)) = s.split_once(':') {
            return (method.to_string(), password.to_string());
        }
    }
    // Plain `method:password` form — the url crate has already percent-decoded these.
    (
        url.username().to_string(),
        url.password().unwrap_or("").to_string(),
    )
}

fn parse_ss(link: &str, sub_id: &str) -> Result<(ProxyNode, Value)> {
    let url = Url::parse(link)?;
    let server = url.host_str().unwrap_or("").to_string();
    let port = url.port().unwrap_or(8388);
    let name = url.fragment().unwrap_or(&server).to_string();
    let params: std::collections::HashMap<String, String> = url.query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    let (method, password) = ss_method_password(link, &url);

    let mut outbound = json!({
        "type": "shadowsocks",
        "tag": name,
        "server": server,
        "server_port": port,
        "method": method,
        "password": password
    });

    // SIP003 plugin (obfs / v2ray-plugin / shadow-tls …). The `plugin` query param is
    // "<name>;<k=v>;<k=v>"; sing-box expects `plugin` (the name) + `plugin_opts` (the
    // remaining semicolon-separated options string). query_pairs() already percent-decodes.
    if let Some(plugin) = params.get("plugin").filter(|p| !p.is_empty()) {
        let (name_part, opts) = match plugin.split_once(';') {
            Some((n, rest)) => (n, rest),
            None => (plugin.as_str(), ""),
        };
        // Normalise the common obfs aliases to the sing-box plugin name.
        let sb_name = match name_part {
            "obfs" | "simple-obfs" => "obfs-local",
            other => other,
        };
        outbound["plugin"] = json!(sb_name);
        if !opts.is_empty() {
            outbound["plugin_opts"] = json!(opts);
        }
    }

    let node = ProxyNode {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.clone(),
        group: "默认".to_string(),
        protocol: "shadowsocks".to_string(),
        server,
        port,
        latency: None,
        download_speed: None,
        is_active: false,
        subscription_id: Some(sub_id.to_string()),
    };

    Ok((node, outbound))
}

fn parse_trojan(link: &str, sub_id: &str) -> Result<(ProxyNode, Value)> {
    let url = Url::parse(link)?;
    let password = url.username().to_string();
    let server = url.host_str().unwrap_or("").to_string();
    let port = url.port().unwrap_or(443);
    let name = url.fragment().unwrap_or(&server).to_string();
    let params: std::collections::HashMap<String, String> = url.query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let sni = params.get("sni").cloned().unwrap_or_else(|| server.clone());
    let insecure = param_insecure(&params);

    let outbound = json!({
        "type": "trojan",
        "tag": name,
        "server": server,
        "server_port": port,
        "password": password,
        "tls": { "enabled": true, "server_name": sni, "insecure": insecure }
    });

    let node = ProxyNode {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.clone(),
        group: "默认".to_string(),
        protocol: "trojan".to_string(),
        server,
        port,
        latency: None,
        download_speed: None,
        is_active: false,
        subscription_id: Some(sub_id.to_string()),
    };

    Ok((node, outbound))
}

fn parse_hysteria2(link: &str, sub_id: &str) -> Result<(ProxyNode, Value)> {
    let normalized = link.replace("hy2://", "hysteria2://");
    let url = Url::parse(&normalized)?;
    let password = url.username().to_string();
    let server = url.host_str().unwrap_or("").to_string();
    let port = url.port().unwrap_or(443);
    let name = url.fragment().unwrap_or(&server).to_string();
    let params: std::collections::HashMap<String, String> = url.query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let sni = params.get("sni").cloned().unwrap_or_else(|| server.clone());
    let insecure = param_insecure(&params);

    let outbound = json!({
        "type": "hysteria2",
        "tag": name,
        "server": server,
        "server_port": port,
        "password": password,
        "tls": { "enabled": true, "server_name": sni, "insecure": insecure }
    });

    let node = ProxyNode {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.clone(),
        group: "默认".to_string(),
        protocol: "hysteria2".to_string(),
        server,
        port,
        latency: None,
        download_speed: None,
        is_active: false,
        subscription_id: Some(sub_id.to_string()),
    };

    Ok((node, outbound))
}

fn parse_tuic(link: &str, sub_id: &str) -> Result<(ProxyNode, Value)> {
    let url = Url::parse(link)?;
    let uuid = url.username().to_string();
    let password = url.password().unwrap_or("").to_string();
    let server = url.host_str().unwrap_or("").to_string();
    let port = url.port().unwrap_or(443);
    let name = url.fragment().unwrap_or(&server).to_string();
    let params: std::collections::HashMap<String, String> = url.query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let sni = params.get("sni").cloned().unwrap_or_else(|| server.clone());
    let insecure = param_insecure(&params);
    let congestion = params.get("congestion_control").cloned()
        .unwrap_or_else(|| "bbr".to_string());
    let udp_relay_mode = params.get("udp_relay_mode").cloned()
        .unwrap_or_else(|| "native".to_string());

    let mut tls = json!({ "enabled": true, "server_name": sni, "insecure": insecure });
    if let Some(alpn) = params.get("alpn") {
        let list: Vec<&str> = alpn.split(',').filter(|s| !s.is_empty()).collect();
        if !list.is_empty() {
            tls["alpn"] = json!(list);
        }
    }

    let outbound = json!({
        "type": "tuic",
        "tag": name,
        "server": server,
        "server_port": port,
        "uuid": uuid,
        "password": password,
        "congestion_control": congestion,
        "udp_relay_mode": udp_relay_mode,
        "tls": tls
    });

    let node = ProxyNode {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.clone(),
        group: "默认".to_string(),
        protocol: "tuic".to_string(),
        server,
        port,
        latency: None,
        download_speed: None,
        is_active: false,
        subscription_id: Some(sub_id.to_string()),
    };

    Ok((node, outbound))
}

fn parse_anytls(link: &str, sub_id: &str) -> Result<(ProxyNode, Value)> {
    let url = Url::parse(link)?;
    // anytls://<password>@host:port  — the password is carried in the userinfo.
    let password = if !url.username().is_empty() {
        url.username().to_string()
    } else {
        url.password().unwrap_or("").to_string()
    };
    let server = url.host_str().unwrap_or("").to_string();
    let port = url.port().unwrap_or(443);
    let name = url.fragment().unwrap_or(&server).to_string();
    let params: std::collections::HashMap<String, String> = url.query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let sni = params.get("sni").cloned().unwrap_or_else(|| server.clone());
    let insecure = param_insecure(&params);

    let outbound = json!({
        "type": "anytls",
        "tag": name,
        "server": server,
        "server_port": port,
        "password": password,
        "tls": { "enabled": true, "server_name": sni, "insecure": insecure }
    });

    let node = ProxyNode {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.clone(),
        group: "默认".to_string(),
        protocol: "anytls".to_string(),
        server,
        port,
        latency: None,
        download_speed: None,
        is_active: false,
        subscription_id: Some(sub_id.to_string()),
    };

    Ok((node, outbound))
}

fn parse_sip008(content: &str, sub_id: &str) -> Result<(Vec<ProxyNode>, Vec<Value>)> {
    let v: Value = serde_json::from_str(content)?;
    let servers = v["servers"].as_array()
        .ok_or_else(|| anyhow!("SIP008: 未找到 servers 字段"))?;

    let mut nodes = Vec::new();
    let mut outbounds = Vec::new();

    for s in servers {
        let name = s["remarks"].as_str()
            .or(s["server"].as_str())
            .unwrap_or("Unknown")
            .to_string();
        let server = s["server"].as_str().unwrap_or("").to_string();
        let port = s["server_port"].as_u64().unwrap_or(0) as u16;
        let method = s["method"].as_str().unwrap_or("aes-128-gcm").to_string();
        let password = s["password"].as_str().unwrap_or("").to_string();

        outbounds.push(json!({
            "type": "shadowsocks",
            "tag": name,
            "server": server,
            "server_port": port,
            "method": method,
            "password": password
        }));

        nodes.push(ProxyNode {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            group: "默认".to_string(),
            protocol: "shadowsocks".to_string(),
            server,
            port,
            latency: None,
            download_speed: None,
            is_active: false,
            subscription_id: Some(sub_id.to_string()),
        });
    }

    Ok((nodes, outbounds))
}

/// Build complete sing-box config.json from outbounds
/// Build the `dns_local` server entry from a user-configured resolver address.
/// `https://…` → DoH, `tls://…` → DoT, anything else → plain UDP. For DoH/DoT a
/// trailing `:port` in the authority is split out into `server_port`.
fn dns_local_server(addr: &str) -> Value {
    let addr = addr.trim();
    let doh_dot = addr
        .strip_prefix("https://")
        .map(|a| ("https", a))
        .or_else(|| addr.strip_prefix("tls://").map(|a| ("tls", a)));

    if let Some((kind, rest)) = doh_dot {
        // Keep only the authority (drop any /dns-query path).
        let authority = rest.split('/').next().unwrap_or(rest);
        let mut entry = json!({ "type": kind, "tag": "dns_local" });
        // Split host:port, but leave IPv6 literals / hosts without a numeric port alone.
        if let Some((host, port)) = authority.rsplit_once(':') {
            if !host.contains(':') {
                if let Ok(p) = port.parse::<u16>() {
                    entry["server"] = json!(host);
                    entry["server_port"] = json!(p);
                    return entry;
                }
            }
        }
        entry["server"] = json!(authority);
        entry
    } else {
        json!({ "type": "udp", "tag": "dns_local", "server": addr })
    }
}

/// Build the outbound list: the `proxy` selector (option order: global "auto" →
/// per-subscription auto groups → custom groups → every node), the `direct`/`block`
/// stubs, the urltest/selector groups, and finally the sanitised concrete node outbounds.
/// Reads the persisted custom proxy-group definitions; otherwise pure in its inputs.
fn build_proxy_outbounds(
    outbounds: &[Value],
    config: &crate::types::AppConfig,
    active_tag: Option<&str>,
    nodes: &[ProxyNode],
) -> Vec<Value> {
    /* Reserved tag for the auto-select (urltest) group. */
    const AUTO_TAG: &str = "auto";

    /* Concrete node tags — one per parsed outbound. */
    let node_tags: Vec<Value> = outbounds.iter()
        .map(|ob| Value::String(ob["tag"].as_str().unwrap_or("").to_string()))
        .collect();
    let has_nodes = !node_tags.is_empty();

    /* Per-subscription auto groups: map each subscription to the node tags that
       both belong to it and actually exist as outbounds. Only subscriptions with at
       least two usable nodes get a dedicated urltest group (a single-node group adds
       no value). BTreeMap keeps group order deterministic across rebuilds. */
    let outbound_tag_set: std::collections::HashSet<&str> = outbounds.iter()
        .filter_map(|ob| ob["tag"].as_str())
        .collect();
    let mut per_sub: std::collections::BTreeMap<String, Vec<Value>> =
        std::collections::BTreeMap::new();
    for node in nodes {
        if let Some(sid) = node.subscription_id.as_deref() {
            if outbound_tag_set.contains(node.name.as_str()) {
                per_sub.entry(sid.to_string())
                    .or_default()
                    .push(Value::String(node.name.clone()));
            }
        }
    }
    /* (group_tag, member_node_tags) for every subscription worth a group. */
    let sub_groups: Vec<(String, Vec<Value>)> = per_sub.into_iter()
        .filter(|(_, members)| members.len() >= 2)
        .map(|(sid, members)| (format!("{}-{}", AUTO_TAG, sid), members))
        .collect();

    /* User-defined custom proxy groups. A group is emitted only when its name does not
       collide with a reserved tag / existing node / sub-group tag (a duplicate outbound
       tag would make sing-box reject the whole config) and it has at least one member
       that actually exists as an outbound. (group_tag, group_type, member_node_tags). */
    let reserved_tags: std::collections::HashSet<String> = {
        let mut s: std::collections::HashSet<String> =
            ["proxy", "direct", "block", AUTO_TAG].iter().map(|t| t.to_string()).collect();
        s.extend(outbound_tag_set.iter().map(|t| t.to_string()));
        s.extend(sub_groups.iter().map(|(t, _)| t.clone()));
        s
    };
    let mut seen_group_tags: std::collections::HashSet<String> = std::collections::HashSet::new();
    let custom_groups: Vec<(String, String, Vec<Value>)> = crate::config::load_proxy_groups()
        .into_iter()
        .filter_map(|g| {
            if reserved_tags.contains(&g.name) || !seen_group_tags.insert(g.name.clone()) {
                return None;
            }
            let members: Vec<Value> = g.nodes.iter()
                .filter(|n| outbound_tag_set.contains(n.as_str()))
                .map(|n| Value::String(n.clone()))
                .collect();
            if members.is_empty() {
                return None;
            }
            let gtype = if g.group_type == "urltest" { "urltest" } else { "selector" };
            Some((g.name, gtype.to_string(), members))
        })
        .collect();

    /* Selector options order: global "auto" → per-subscription autos → every node. */
    let mut selector_outbounds: Vec<Value> = Vec::new();
    if has_nodes {
        selector_outbounds.push(Value::String(AUTO_TAG.to_string()));
    }
    for (tag, _) in &sub_groups {
        selector_outbounds.push(Value::String(tag.clone()));
    }
    for (tag, _, _) in &custom_groups {
        selector_outbounds.push(Value::String(tag.clone()));
    }
    selector_outbounds.extend(node_tags.iter().cloned());
    /* With no nodes configured yet the selector would otherwise be empty, which makes
       sing-box reject the config. Fall back to "direct" so a persistent idle core can
       still start (and "proxy" simply means direct until the user adds nodes). */
    if selector_outbounds.is_empty() {
        selector_outbounds.push(Value::String("direct".to_string()));
    }

    /* Default selection priority:
         1. caller-supplied active_tag (a concrete node tag or "auto")
         2. "auto" when nodes exist (best zero-config experience)
         3. first available option */
    let mut selected: String = match active_tag {
        Some(t) if !t.is_empty() => t.to_string(),
        _ if has_nodes => AUTO_TAG.to_string(),
        _ => selector_outbounds.first().and_then(|v| v.as_str()).unwrap_or("").to_string(),
    };
    /* Guard against a stale active_tag (e.g. the node was removed after it was last
       selected): fall back to a valid option so sing-box never sees an unknown
       default, which would abort startup. */
    if !selector_outbounds.iter().any(|v| v.as_str() == Some(selected.as_str())) {
        selected = if has_nodes { AUTO_TAG.to_string() } else { "direct".to_string() };
    }

    // Sanitize proxy outbounds: remove any transport field whose type is "tcp"
    // (tcp is the default in sing-box ? the field must be absent, not explicitly set).
    // This also fixes outbounds cached before this rule was enforced in the parser.
    let clean_outbounds: Vec<Value> = outbounds.iter().map(|ob| {
        let mut ob = ob.clone();
        let is_tcp_transport = ob.get("transport")
            .and_then(|t| t.get("type"))
            .and_then(|t| t.as_str())
            .map(|t| t == "tcp")
            .unwrap_or(false);
        if is_tcp_transport {
            if let Some(map) = ob.as_object_mut() {
                map.remove("transport");
            }
        }
        ob
    }).collect();

    let mut all_outbounds = vec![
        json!({
            "type": "selector",
            "tag": "proxy",
            "outbounds": selector_outbounds,
            "default": selected
        }),
        json!({ "type": "direct", "tag": "direct" }),
        json!({ "type": "block", "tag": "block" }),
    ];

    /* URLTest groups: the core health-checks the member nodes and routes through the
       lowest-latency one, re-evaluating on `interval`. This gives the app
       Clash.Meta-style "Auto" behaviour and fixes the "locked to a slow node"
       problem. Probe URL / interval / tolerance are user-configurable.
         • global "auto"      → all nodes
         • "auto-<sub.id>"    → only that subscription's nodes (multi-airport setups) */
    let test_url = if config.auto_test_url.trim().is_empty() {
        "https://www.gstatic.com/generate_204".to_string()
    } else {
        config.auto_test_url.trim().to_string()
    };
    let interval = format!("{}m", config.auto_test_interval.max(1));
    let tolerance = config.auto_tolerance;
    let urltest_group = |tag: &str, members: &[Value]| -> Value {
        json!({
            "type": "urltest",
            "tag": tag,
            "outbounds": members,
            "url": test_url.clone(),
            "interval": interval.clone(),
            "tolerance": tolerance,
            "idle_timeout": "30m"
        })
    };
    if has_nodes {
        all_outbounds.push(urltest_group(AUTO_TAG, &node_tags));
    }
    for (tag, members) in &sub_groups {
        all_outbounds.push(urltest_group(tag, members));
    }
    /* User-defined custom groups: urltest (auto by latency) or selector (manual pick,
       defaulting to its first member). */
    for (tag, gtype, members) in &custom_groups {
        if gtype == "urltest" {
            all_outbounds.push(urltest_group(tag, members));
        } else {
            all_outbounds.push(json!({
                "type": "selector",
                "tag": tag,
                "outbounds": members,
                "default": members[0]
            }));
        }
    }
    all_outbounds.extend_from_slice(&clean_outbounds);
    all_outbounds
}

/// Local-resolver (real-IP) DNS rules, in priority order: proxy-server hostnames → CN-core
/// suffixes (reliable even without geosite-cn.srs) → user DIRECT domain matchers →
/// geosite-cn → A/AAAA fall-through to fake-ip. Non-A/AAAA queries fall through to
/// `final: dns_local` at the call site.
fn build_dns_rules(
    server_domains: Vec<Value>,
    cn_core_domains: &[Value],
    user_rules: &[crate::rules::RouteRule],
) -> Vec<Value> {
    // Build domain_suffix entries from user-defined DIRECT rules so that those
    // domains are also resolved via dns_local (real IP), not fakeip.
    let mut user_direct_domain_suffixes: Vec<Value> = Vec::new();
    let mut user_direct_domains: Vec<Value> = Vec::new();
    for rule in user_rules.iter().filter(|r| r.enabled && r.action == crate::rules::RuleAction::Direct) {
        for d in &rule.domain_suffix {
            user_direct_domain_suffixes.push(Value::String(d.clone()));
        }
        for d in &rule.domain {
            user_direct_domains.push(Value::String(d.clone()));
        }
    }

    let mut dns_rules: Vec<Value> = Vec::new();
    if !server_domains.is_empty() {
        dns_rules.push(json!({ "domain": server_domains, "server": "dns_local" }));
    }
    // Explicit CN core domains — reliable even without geosite-cn.srs
    dns_rules.push(json!({ "domain_suffix": cn_core_domains, "server": "dns_local" }));
    // User-defined direct rules
    if !user_direct_domain_suffixes.is_empty() || !user_direct_domains.is_empty() {
        let mut entry = serde_json::Map::new();
        if !user_direct_domain_suffixes.is_empty() {
            entry.insert("domain_suffix".into(), json!(user_direct_domain_suffixes));
        }
        if !user_direct_domains.is_empty() {
            entry.insert("domain".into(), json!(user_direct_domains));
        }
        entry.insert("server".into(), json!("dns_local"));
        dns_rules.push(Value::Object(entry));
    }
    dns_rules.push(json!({ "rule_set": "geosite-cn", "server": "dns_local" }));
    dns_rules.push(json!({
        "query_type": ["A", "AAAA"],
        "server": "dns_fakeip"
    }));
    dns_rules
}

/// Build the `route.rules` array plus the remote `rule_set` definitions contributed by
/// user rule-set providers, returned as `(route_rules, provider_rule_sets)`. Order matters:
/// sniff/dns/private/clash-mode → WeChat-direct → CN-core direct → user rules → provider
/// rule-sets → broad geosite-cn/geoip-cn catch-alls.
fn build_route_rules(
    cn_core_domains: &[Value],
    user_rules: &[crate::rules::RouteRule],
    rule_providers: &[crate::rules::RuleProvider],
) -> (Vec<Value>, Vec<Value>) {
    let mut route_rules: Vec<Value> = vec![
        json!({ "action": "sniff" }),
        json!({ "protocol": ["dns"], "action": "hijack-dns" }),
        json!({ "ip_is_private": true, "outbound": "direct" }),
        json!({ "clash_mode": "Direct", "outbound": "direct" }),
        json!({ "clash_mode": "Global", "outbound": "proxy" }),
        // WeChat process — route ALL WeChat traffic direct in TUN mode so that
        // screenshot translation, voice messages and other CN-API features work
        // without being affected by proxy routing or fake-ip DNS assignment.
        // process_name is only evaluated in TUN mode (ignored for mixed-inbound).
        json!({
            "process_name": crate::cn_direct::WECHAT_PROCESSES,
            "outbound": "direct"
        }),
        // Explicit CN-core domains — reliable direct path regardless of geosite-cn.srs
        json!({ "domain_suffix": cn_core_domains, "outbound": "direct" }),
    ];

    // Inject user-defined routing rules (domain/keyword/process matchers only).
    for rule in user_rules.iter().filter(|r| r.enabled) {
        let mut obj = serde_json::Map::new();
        if !rule.domain.is_empty() {
            obj.insert("domain".into(), json!(rule.domain));
        }
        if !rule.domain_suffix.is_empty() {
            obj.insert("domain_suffix".into(), json!(rule.domain_suffix));
        }
        if !rule.domain_keyword.is_empty() {
            obj.insert("domain_keyword".into(), json!(rule.domain_keyword));
        }
        if !rule.process_name.is_empty() {
            obj.insert("process_name".into(), json!(rule.process_name));
        }
        // Port rules
        if !rule.port.is_empty() {
            let ports: Vec<Value> = rule.port.iter()
                .flat_map(|p| {
                    if p.contains('-') {
                        let parts: Vec<&str> = p.split('-').collect();
                        if parts.len() == 2 {
                            if let (Ok(s), Ok(e)) = (parts[0].parse::<u16>(), parts[1].parse::<u16>()) {
                                return (s..=e).map(|n| json!(n)).collect::<Vec<_>>();
                            }
                        }
                        vec![]
                    } else if let Ok(n) = p.parse::<u16>() {
                        vec![json!(n)]
                    } else {
                        vec![]
                    }
                })
                .collect();
            if !ports.is_empty() {
                obj.insert("port".into(), json!(ports));
            }
        }
        if !obj.is_empty() {
            let outbound = match rule.action {
                crate::rules::RuleAction::Proxy => "proxy",
                crate::rules::RuleAction::Direct => "direct",
                crate::rules::RuleAction::Block => "block",
                crate::rules::RuleAction::Dns => continue, // handled by hijack-dns
            };
            obj.insert("outbound".into(), json!(outbound));
            route_rules.push(Value::Object(obj));
        }
    }

    // User-added remote rule-set providers. Each enabled provider contributes one
    // remote rule_set entry (downloaded through the proxy) and one route rule mapping
    // its matches to the chosen outbound. Placed before the broad CN catch-alls so a
    // user provider can override the default geosite-cn/geoip-cn direct routing.
    let mut provider_rule_sets: Vec<Value> = Vec::new();
    for p in rule_providers.iter().filter(|p| p.enabled && !p.url.is_empty()) {
        let tag = format!("rp-{}", p.id);
        let outbound = match p.action {
            crate::rules::RuleAction::Proxy => "proxy",
            crate::rules::RuleAction::Direct => "direct",
            crate::rules::RuleAction::Block => "block",
            crate::rules::RuleAction::Dns => continue,
        };
        provider_rule_sets.push(json!({
            "type": "remote",
            "tag": tag,
            "format": p.format,
            "url": p.url,
            "download_detour": "proxy",
            "update_interval": "1d"
        }));
        route_rules.push(json!({ "rule_set": [tag], "outbound": outbound }));
    }

    // Broad CN catch-alls (geosite/geoip rule sets)
    route_rules.push(json!({ "rule_set": ["geosite-cn"], "outbound": "direct" }));
    route_rules.push(json!({ "rule_set": ["geoip-cn"], "outbound": "direct" }));

    (route_rules, provider_rule_sets)
}

/// Local inbounds: the always-present mixed inbound (HTTP+SOCKS on one port), plus
/// dedicated http/socks inbounds only when their port differs from the mixed port (and
/// each other) — reusing a port would make sing-box fail to start.
fn build_inbounds(config: &crate::types::AppConfig) -> Vec<Value> {
    // Listen address: bind to all interfaces when LAN sharing is enabled, otherwise
    // stay loopback-only. Applies to every local inbound (mixed/http/socks).
    let listen_addr = if config.allow_lan { "0.0.0.0" } else { "127.0.0.1" };

    let mut inbounds: Vec<Value> = vec![json!({
        "type": "mixed",
        "tag": "mixed-in",
        "listen": listen_addr,
        "listen_port": config.mixed_port,
        "set_system_proxy": false
    })];
    if config.http_port != 0 && config.http_port != config.mixed_port {
        inbounds.push(json!({
            "type": "http",
            "tag": "http-in",
            "listen": listen_addr,
            "listen_port": config.http_port
        }));
    }
    if config.socks_port != 0
        && config.socks_port != config.mixed_port
        && config.socks_port != config.http_port
    {
        inbounds.push(json!({
            "type": "socks",
            "tag": "socks-in",
            "listen": listen_addr,
            "listen_port": config.socks_port
        }));
    }
    inbounds
}

/// Build the TUN inbound. An IPv6 address (and thus IPv6 routes) is only assigned when
/// IPv6 is enabled. On Windows a unique per-start interface name avoids WinTun collisions
/// with adapters orphaned by a previous crash.
fn build_tun_inbound(config: &crate::types::AppConfig) -> Value {
    // Only assign an IPv6 TUN address (and thus add IPv6 routes) when IPv6 is enabled;
    // otherwise stay IPv4-only so no v6 traffic is captured by the tunnel.
    let tun_address = if config.enable_ipv6 {
        json!(["172.19.0.1/30", "fdfe:dcba:9876::1/126"])
    } else {
        json!(["172.19.0.1/30"])
    };
    let mut tun_in = json!({
        "type": "tun",
        "tag": "tun-in",
        "address": tun_address,
        "mtu": 9000,
        "auto_route": true,
        "strict_route": true,
        "stack": "system"
    });

    // On Windows, use a unique interface name per start. If a previous run crashed
    // and left an orphaned adapter behind, a fresh name avoids the WinTun "Cannot
    // create a file when that file already exists" failure entirely. Old
    // "skylark-tun*" adapters are cleaned up before start by cleanup_stale_tun_adapter().
    //
    // On macOS/Linux the TUN device must be named "utunN"/"tunN" by the kernel, so we
    // omit interface_name and let sing-box pick a valid one automatically.
    #[cfg(target_os = "windows")]
    {
        let unique_suffix = uuid::Uuid::new_v4().simple().to_string();
        let interface_name = format!("skylark-tun-{}", &unique_suffix[..6]);
        tun_in["interface_name"] = json!(interface_name);
    }

    tun_in
}

/// Assemble the complete sing-box config: log + DNS + inbounds + outbounds + route +
/// experimental(clash_api/cache). Orchestrates the `build_*` helpers above; the only
/// pieces kept inline are the small rule-set definitions, the proxy-server hostname /
/// CN-core domain lists they feed, and the final JSON assembly.
pub fn build_singbox_config(
    outbounds: &[Value],
    config: &crate::types::AppConfig,
    active_tag: Option<&str>,
    nodes: &[ProxyNode],
) -> Value {
    let all_outbounds = build_proxy_outbounds(outbounds, config, active_tag, nodes);

    // ── Rule-sets ──────────────────────────────────────────────────────
    // Prefer the locally bundled .srs files (copied to the app data dir on
    // startup). They work offline and in regions where jsDelivr/GitHub are
    // blocked. If a file is somehow missing, fall back to downloading it
    // THROUGH THE PROXY (download_detour: "proxy"), which is reachable once the
    // tunnel is up — never via "direct", which fails behind the GFW.
    let rule_set_entry = |tag: &str, file: &str, url: &str| -> Value {
        let path = crate::config::rule_sets_dir().join(file);
        if path.exists() {
            json!({
                "type": "local",
                "tag": tag,
                "format": "binary",
                "path": path.to_string_lossy().replace('\\', "/")
            })
        } else {
            json!({
                "type": "remote",
                "tag": tag,
                "format": "binary",
                "url": url,
                "download_detour": "proxy",
                "update_interval": "7d"
            })
        }
    };
    let geosite_cn_rs = rule_set_entry(
        "geosite-cn", "geosite-cn.srs",
        "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-cn.srs",
    );
    let geoip_cn_rs = rule_set_entry(
        "geoip-cn", "geoip-cn.srs",
        "https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/geoip-cn.srs",
    );

    // Proxy server entry hostnames MUST resolve to their real IP (never fake-ip),
    // otherwise the dialer cannot reach the node. Collect non-IP server fields.
    let server_domains: Vec<Value> = outbounds.iter()
        .filter_map(|ob| ob.get("server").and_then(|s| s.as_str()))
        .filter(|s| s.parse::<std::net::IpAddr>().is_err())
        .map(|s| Value::String(s.to_string()))
        .collect();

    // DNS routing rules (order matters — first match wins):
    //   1. proxy node hostnames  → real IP (so the dialer can reach the node)
    //   2. explicit domestic domains (Tencent/WeChat/Alibaba/etc.) → real IP
    //      This acts as a reliable fallback even when geosite-cn.srs is missing
    //      (e.g. first launch before the file is downloaded). Without this rule,
    //      WeChat screenshot translation and similar CN-API callers would receive
    //      a fake IP and be proxied through a foreign exit node, causing Tencent
    //      servers to reject the request due to IP geo-checking.
    //   3. user-defined direct rules with domain matchers → real IP
    //   4. CN domains via geosite-cn rule set → real IP via the local resolver
    //   5. EVERYTHING ELSE A/AAAA → fake IP, so the EXIT node performs the real
    //      lookup (correct CDN edge ⇒ full node speed, no per-connection DoT).
    // Non-A/AAAA queries (HTTPS/SVCB/PTR…) fall through to `final: dns_local`.
    // Single source of truth shared with proxy.rs (see crate::cn_direct).
    let cn_core_domains: Vec<Value> = crate::cn_direct::CN_DIRECT_SUFFIXES
        .iter()
        .map(|s| Value::String(s.to_string()))
        .collect();

    let user_rules = crate::rules::load_rules();
    let rule_providers = crate::rules::load_rule_providers();

    let dns_rules = build_dns_rules(server_domains, &cn_core_domains, &user_rules);
    let (route_rules, provider_rule_sets) =
        build_route_rules(&cn_core_domains, &user_rules, &rule_providers);
    let inbounds = build_inbounds(config);

    // Combine the built-in CN rule sets with any user-added remote providers.
    let mut all_rule_sets: Vec<Value> = vec![geosite_cn_rs, geoip_cn_rs];
    all_rule_sets.extend(provider_rule_sets);

    // DNS: local resolver is user-configurable (UDP / DoH / DoT). IPv6 support is gated
    // behind `enable_ipv6` — when off we stay strictly IPv4 (previous behaviour); when on
    // we resolve dual-stack (prefer IPv4) and hand out fake IPv6 addresses too.
    let dns_local_entry = dns_local_server(&config.dns_local);
    let mut dns_fakeip = json!({
        "type": "fakeip",
        "tag": "dns_fakeip",
        "inet4_range": "198.18.0.0/15"
    });
    if config.enable_ipv6 {
        dns_fakeip["inet6_range"] = json!("fc00::/18");
    }
    let dns_strategy = if config.enable_ipv6 { "prefer_ipv4" } else { "ipv4_only" };

    let mut cfg = json!({
        "log": { "level": config.log_level, "timestamp": true },
        "dns": {
            "servers": [dns_local_entry, dns_fakeip],
            "rules": dns_rules,
            "final": "dns_local",
            "strategy": dns_strategy,
            "independent_cache": true
        },
        "inbounds": inbounds,
        "outbounds": all_outbounds,
        "route": {
            "default_domain_resolver": "dns_local",
            "rules": route_rules,
            "rule_set": all_rule_sets,
            "final": "proxy",
            "auto_detect_interface": true
        },
        "experimental": {
            "clash_api": {
                "external_controller": format!("127.0.0.1:{}", config.api_port),
                "external_ui": "ui",
                // Random per-install secret so other local processes can't drive the core
                // via the unauthenticated Clash API. Every in-app caller sends it as a
                // Bearer token (see crate::config::api_secret).
                "secret": crate::config::api_secret(),
                // Startup routing mode. Switching rule/global/direct at runtime is done
                // live via `PATCH /configs` (no core restart) and persisted to the cache
                // file; this only sets the mode for a fresh start with no cached value.
                "default_mode": match config.proxy_mode {
                    crate::types::ProxyMode::Global => "Global",
                    crate::types::ProxyMode::Direct => "Direct",
                    _ => "Rule",
                }
            },
            /* Persist routing/fake-ip state across restarts. store_fakeip keeps the
               domain↔fake-IP mapping stable so long-lived connections survive a
               proxy restart instead of resolving to a stale address. */
            "cache_file": {
                "enabled": true,
                "store_fakeip": true
            }
        }
    });

    if config.tun_enabled {
        let tun_in = build_tun_inbound(config);
        cfg["inbounds"].as_array_mut().unwrap().push(tun_in);
    }

    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    /** Encode a raw JSON payload as a `vmess://<base64>` link. */
    fn vmess_link(payload: &str) -> String {
        format!("vmess://{}", general_purpose::STANDARD.encode(payload))
    }

    #[test]
    fn detect_sub_type_recognises_known_formats() {
        assert_eq!(detect_sub_type("proxies:\n  - name: a", ""), SubType::Clash);
        assert_eq!(detect_sub_type("vless://uuid@host:443#n", ""), SubType::V2ray);
        let sip008 = r#"{"version":1,"servers":[{"server":"h","server_port":8388,"method":"aes-256-gcm","password":"p"}]}"#;
        assert_eq!(detect_sub_type(sip008, ""), SubType::Sip008);
        assert_eq!(detect_sub_type("just some random text", ""), SubType::Unknown);
    }

    #[test]
    fn parse_vmess_ws_tls() {
        let payload = r#"{"v":"2","ps":"VM1","add":"example.com","port":"443","id":"11111111-1111-1111-1111-111111111111","aid":"0","net":"ws","path":"/ray","host":"cdn.example.com","tls":"tls"}"#;
        let (node, ob) = parse_vmess(&vmess_link(payload), "sub1").unwrap();

        assert_eq!(node.protocol, "vmess");
        assert_eq!(node.name, "VM1");
        assert_eq!(node.server, "example.com");
        assert_eq!(node.port, 443u16);
        assert_eq!(node.subscription_id.as_deref(), Some("sub1"));

        assert_eq!(ob["type"], "vmess");
        assert_eq!(ob["tag"], "VM1");
        assert_eq!(ob["server_port"], 443);
        assert_eq!(ob["uuid"], "11111111-1111-1111-1111-111111111111");
        assert_eq!(ob["transport"]["type"], "ws");
        assert_eq!(ob["transport"]["path"], "/ray");
        assert_eq!(ob["transport"]["headers"]["Host"], "cdn.example.com");
        assert_eq!(ob["tls"]["enabled"], true);
        // sni is absent in the payload, so it falls back to the host header value
        assert_eq!(ob["tls"]["server_name"], "cdn.example.com");
    }

    #[test]
    fn parse_vmess_plain_tcp_has_no_transport_or_tls() {
        let payload = r#"{"ps":"VM2","add":"1.2.3.4","port":443,"id":"x","net":"tcp"}"#;
        let (_node, ob) = parse_vmess(&vmess_link(payload), "s").unwrap();
        assert!(ob["transport"].is_null());
        assert!(ob["tls"].is_null());
    }

    #[test]
    fn parse_vless_reality_omits_insecure() {
        let link = "vless://22222222-2222-2222-2222-222222222222@host.net:443?security=reality&pbk=PUBKEY&sid=ab12&sni=www.example.com&fp=chrome&flow=xtls-rprx-vision#VL-R";
        let (node, ob) = parse_vless(link, "s").unwrap();

        assert_eq!(node.protocol, "vless");
        assert_eq!(ob["type"], "vless");
        assert_eq!(ob["uuid"], "22222222-2222-2222-2222-222222222222");
        assert_eq!(ob["flow"], "xtls-rprx-vision");
        assert_eq!(ob["tls"]["enabled"], true);
        assert_eq!(ob["tls"]["server_name"], "www.example.com");
        assert_eq!(ob["tls"]["utls"]["fingerprint"], "chrome");
        assert_eq!(ob["tls"]["reality"]["enabled"], true);
        assert_eq!(ob["tls"]["reality"]["public_key"], "PUBKEY");
        assert_eq!(ob["tls"]["reality"]["short_id"], "ab12");
        // Reality verifies via the public key, so a plain `insecure` flag must be absent.
        assert!(ob["tls"]["insecure"].is_null());
    }

    #[test]
    fn parse_ss_plaintext_userinfo_fallback() {
        // Plain `method:password` userinfo (no base64): the ':' makes base64 decoding
        // fail, so the parser falls back to the url-provided username/password.
        let link = "ss://aes-256-gcm:secretpass@ss.example.com:8388#SS-Node";
        let (node, ob) = parse_ss(link, "s").unwrap();

        assert_eq!(node.protocol, "shadowsocks");
        assert_eq!(ob["type"], "shadowsocks");
        assert_eq!(ob["method"], "aes-256-gcm");
        assert_eq!(ob["password"], "secretpass");
        assert_eq!(ob["server"], "ss.example.com");
        assert_eq!(ob["server_port"], 8388);
    }

    #[test]
    fn parse_ss_base64_standard_padded_userinfo() {
        // Standard SIP002: userinfo = base64(method:password), with padding. The `url`
        // crate percent-encodes the '=' padding, so the parser must read the raw blob
        // from the link itself (regression test for the bug found via Q1).
        let userinfo = general_purpose::STANDARD.encode("aes-256-gcm:secretpass");
        assert!(userinfo.ends_with("=="), "fixture should be padded");
        let link = format!("ss://{}@ss.example.com:8388#SS", userinfo);
        let (_node, ob) = parse_ss(&link, "s").unwrap();

        assert_eq!(ob["method"], "aes-256-gcm");
        assert_eq!(ob["password"], "secretpass");
        assert_eq!(ob["server"], "ss.example.com");
        assert_eq!(ob["server_port"], 8388);
    }

    #[test]
    fn parse_ss_base64_urlsafe_unpadded_userinfo() {
        let userinfo = general_purpose::URL_SAFE_NO_PAD.encode("chacha20-ietf-poly1305:p@ss-word");
        let link = format!("ss://{}@h.example.com:443#N", userinfo);
        let (_node, ob) = parse_ss(&link, "s").unwrap();

        assert_eq!(ob["method"], "chacha20-ietf-poly1305");
        assert_eq!(ob["password"], "p@ss-word");
    }

    #[test]
    fn parse_ss_with_obfs_plugin() {
        // plugin param is percent-encoded in the wire link; query_pairs decodes it.
        let link = "ss://aes-256-gcm:pw@h.example.com:8388?plugin=obfs-local%3Bobfs%3Dhttp%3Bobfs-host%3Dwww.bing.com#N";
        let (_node, ob) = parse_ss(link, "s").unwrap();

        assert_eq!(ob["plugin"], "obfs-local");
        assert_eq!(ob["plugin_opts"], "obfs=http;obfs-host=www.bing.com");
    }

    #[test]
    fn parse_ss_obfs_alias_normalised() {
        let link = "ss://aes-256-gcm:pw@h.example.com:8388?plugin=obfs%3Bobfs%3Dtls#N";
        let (_node, ob) = parse_ss(link, "s").unwrap();
        // "obfs" / "simple-obfs" normalise to the sing-box plugin name "obfs-local".
        assert_eq!(ob["plugin"], "obfs-local");
        assert_eq!(ob["plugin_opts"], "obfs=tls");
    }

    #[test]
    fn parse_trojan_honours_allow_insecure() {
        let link = "trojan://mypassword@trojan.example.com:443?sni=sni.example.com&allowInsecure=1#TR";
        let (node, ob) = parse_trojan(link, "s").unwrap();

        assert_eq!(node.protocol, "trojan");
        assert_eq!(ob["type"], "trojan");
        assert_eq!(ob["password"], "mypassword");
        assert_eq!(ob["tls"]["server_name"], "sni.example.com");
        assert_eq!(ob["tls"]["insecure"], true);
    }

    #[test]
    fn parse_hysteria2_normalises_hy2_scheme() {
        let link = "hy2://pw@hy.example.com:8443?sni=sni.example.com#HY";
        let (node, ob) = parse_hysteria2(link, "s").unwrap();

        assert_eq!(node.protocol, "hysteria2");
        assert_eq!(ob["type"], "hysteria2");
        assert_eq!(ob["server_port"], 8443);
        assert_eq!(ob["password"], "pw");
        assert_eq!(ob["tls"]["server_name"], "sni.example.com");
        // No allowInsecure param → verification stays on.
        assert_eq!(ob["tls"]["insecure"], false);
    }

    #[test]
    fn parse_tuic_splits_alpn_list() {
        let link = "tuic://uuid-1:pw@tuic.example.com:443?alpn=h3,h2&congestion_control=bbr#TU";
        let (node, ob) = parse_tuic(link, "s").unwrap();

        assert_eq!(node.protocol, "tuic");
        assert_eq!(ob["type"], "tuic");
        assert_eq!(ob["uuid"], "uuid-1");
        assert_eq!(ob["password"], "pw");
        assert_eq!(ob["congestion_control"], "bbr");
        assert_eq!(ob["udp_relay_mode"], "native");
        assert_eq!(ob["tls"]["alpn"], json!(["h3", "h2"]));
    }

    #[test]
    fn parse_anytls_basic() {
        let link = "anytls://pw@anytls.example.com:443?sni=sni.example.com#AT";
        let (node, ob) = parse_anytls(link, "s").unwrap();
        assert_eq!(node.protocol, "anytls");
        assert_eq!(ob["type"], "anytls");
        assert_eq!(ob["password"], "pw");
        assert_eq!(ob["tls"]["server_name"], "sni.example.com");
    }

    #[test]
    fn parse_node_link_rejects_unknown_scheme() {
        assert!(parse_node_link("ssr://whatever", "s").is_err());
    }

    /// Characterization smoke test for the config generator: locks the structural
    /// invariants every generated config must satisfy, independent of on-disk user
    /// rules / proxy groups. F2 / F3 may extend the contents but must keep this shape.
    #[test]
    fn build_singbox_config_has_core_structure() {
        let outbound = json!({
            "type": "vmess",
            "tag": "NodeA",
            "server": "example.com",
            "server_port": 443,
            "uuid": "11111111-1111-1111-1111-111111111111",
            "alter_id": 0,
            "security": "auto"
        });
        let cfg = crate::types::AppConfig::default();
        let result = build_singbox_config(&[outbound], &cfg, None, &[]);

        // Top-level sections present and well-typed.
        assert!(result["dns"].is_object(), "dns section missing");
        assert!(result["inbounds"].is_array(), "inbounds must be an array");
        assert!(result["route"]["rules"].is_array(), "route.rules must be an array");

        let outbounds = result["outbounds"].as_array().expect("outbounds array");

        // The parsed node is passed through verbatim.
        assert!(
            outbounds.iter().any(|o| o["tag"] == "NodeA"),
            "node outbound not propagated"
        );
        // The primary selector and the built-in direct outbound always exist.
        assert!(
            outbounds.iter().any(|o| o["tag"] == "proxy" && o["type"] == "selector"),
            "missing `proxy` selector"
        );
        assert!(
            outbounds.iter().any(|o| o["tag"] == "direct"),
            "missing `direct` outbound"
        );

        // Clash API is bound to loopback on the configured port and guarded by a secret.
        let clash = &result["experimental"]["clash_api"];
        assert_eq!(
            clash["external_controller"],
            format!("127.0.0.1:{}", cfg.api_port)
        );
        assert!(
            clash["secret"].as_str().map(|s| !s.is_empty()).unwrap_or(false),
            "clash_api.secret must be a non-empty string"
        );
    }

    /** Locate the DNS server entry with the given tag in a generated config. */
    fn dns_server<'a>(cfg: &'a Value, tag: &str) -> &'a Value {
        cfg["dns"]["servers"]
            .as_array()
            .expect("dns.servers array")
            .iter()
            .find(|s| s["tag"] == tag)
            .unwrap_or(&Value::Null)
    }

    #[test]
    fn build_config_dns_defaults_ipv4_only() {
        let cfg = crate::types::AppConfig::default();
        let result = build_singbox_config(&[], &cfg, None, &[]);

        assert_eq!(result["dns"]["strategy"], "ipv4_only");
        let local = dns_server(&result, "dns_local");
        assert_eq!(local["type"], "udp");
        assert_eq!(local["server"], "223.5.5.5");
        // No IPv6 fake range when IPv6 is disabled.
        assert!(dns_server(&result, "dns_fakeip")["inet6_range"].is_null());
    }

    #[test]
    fn build_config_ipv6_enables_dual_stack() {
        let mut cfg = crate::types::AppConfig::default();
        cfg.enable_ipv6 = true;
        let result = build_singbox_config(&[], &cfg, None, &[]);

        assert_eq!(result["dns"]["strategy"], "prefer_ipv4");
        assert!(
            !dns_server(&result, "dns_fakeip")["inet6_range"].is_null(),
            "fakeip should expose an inet6_range when IPv6 is on"
        );
    }

    #[test]
    fn build_config_custom_doh_resolver() {
        let mut cfg = crate::types::AppConfig::default();
        cfg.dns_local = "https://1.1.1.1/dns-query".to_string();
        let result = build_singbox_config(&[], &cfg, None, &[]);

        let local = dns_server(&result, "dns_local");
        assert_eq!(local["type"], "https");
        assert_eq!(local["server"], "1.1.1.1");
    }

    #[test]
    fn dns_local_server_parses_schemes() {
        assert_eq!(dns_local_server("223.5.5.5")["type"], "udp");
        assert_eq!(dns_local_server("223.5.5.5")["server"], "223.5.5.5");

        let doh = dns_local_server("https://dns.google:443/dns-query");
        assert_eq!(doh["type"], "https");
        assert_eq!(doh["server"], "dns.google");
        assert_eq!(doh["server_port"], 443);

        let dot = dns_local_server("tls://8.8.8.8");
        assert_eq!(dot["type"], "tls");
        assert_eq!(dot["server"], "8.8.8.8");
    }

    // ── Extracted build_* helper unit tests ────────────────────────────
    // These lock down the pure sub-builders directly (finer-grained than the
    // end-to-end `build_singbox_config_*` tests above).

    #[test]
    fn build_inbounds_collapses_duplicate_ports() {
        // http/socks sharing the mixed port must NOT create extra inbounds
        // (sing-box rejects two inbounds on one port).
        let mut cfg = crate::types::AppConfig::default();
        cfg.mixed_port = 7890;
        cfg.http_port = 7890;
        cfg.socks_port = 7890;
        let inbounds = build_inbounds(&cfg);
        assert_eq!(inbounds.len(), 1, "only the mixed inbound should remain");
        assert_eq!(inbounds[0]["tag"], "mixed-in");
    }

    #[test]
    fn build_inbounds_distinct_ports_and_lan() {
        let mut cfg = crate::types::AppConfig::default();
        cfg.mixed_port = 7890;
        cfg.http_port = 7891;
        cfg.socks_port = 7892;
        cfg.allow_lan = true;
        let inbounds = build_inbounds(&cfg);
        assert_eq!(inbounds.len(), 3, "mixed + http + socks");
        assert!(inbounds.iter().all(|i| i["listen"] == "0.0.0.0"),
            "allow_lan must bind every inbound to 0.0.0.0");
        assert!(inbounds.iter().any(|i| i["tag"] == "http-in" && i["listen_port"] == 7891));
        assert!(inbounds.iter().any(|i| i["tag"] == "socks-in" && i["listen_port"] == 7892));
    }

    #[test]
    fn build_dns_rules_priority_and_fakeip_last() {
        let server_domains = vec![Value::String("node.example.com".into())];
        let cn = vec![Value::String("qq.com".into())];
        let rules = build_dns_rules(server_domains, &cn, &[]);
        // First rule routes proxy-server hostnames to the real resolver.
        assert_eq!(rules[0]["domain"][0], "node.example.com");
        assert_eq!(rules[0]["server"], "dns_local");
        // Last rule is the A/AAAA → fake-ip catch-all.
        let last = rules.last().unwrap();
        assert_eq!(last["server"], "dns_fakeip");
        assert_eq!(last["query_type"][0], "A");
    }

    #[test]
    fn build_route_rules_frames_with_sniff_and_catchalls() {
        let cn = vec![Value::String("qq.com".into())];
        let (rules, providers) = build_route_rules(&cn, &[], &[]);
        assert_eq!(rules[0]["action"], "sniff", "first rule must be sniff");
        // The two broad CN catch-alls always close the list.
        let n = rules.len();
        assert_eq!(rules[n - 2]["rule_set"][0], "geosite-cn");
        assert_eq!(rules[n - 1]["rule_set"][0], "geoip-cn");
        assert!(providers.is_empty(), "no providers ⇒ no provider rule-sets");
    }

    #[test]
    fn build_proxy_outbounds_falls_back_to_direct_when_empty() {
        let cfg = crate::types::AppConfig::default();
        let obs = build_proxy_outbounds(&[], &cfg, None, &[]);
        let proxy = obs.iter().find(|o| o["tag"] == "proxy").expect("proxy selector");
        // With no nodes the selector must still be valid: a lone "direct" option.
        assert_eq!(proxy["default"], "direct");
        assert_eq!(proxy["outbounds"][0], "direct");
        assert!(obs.iter().any(|o| o["tag"] == "direct"));
        assert!(obs.iter().any(|o| o["tag"] == "block"));
    }

    /** Parse a YAML fragment into the `YamlValue` a Clash proxy entry would be. */
    fn yaml_proxy(src: &str) -> YamlValue {
        serde_yaml::from_str(src).expect("valid yaml")
    }

    #[test]
    fn clash_ss_obfs_plugin_mapped() {
        let p = yaml_proxy(
            "type: ss\nserver: 1.2.3.4\nport: 8388\ncipher: aes-256-gcm\npassword: pw\nplugin: obfs\nplugin-opts:\n  mode: tls\n  host: bing.com\n",
        );
        let ob = clash_yaml_proxy_to_singbox(&p, "X").expect("ss outbound");
        assert_eq!(ob["type"], "shadowsocks");
        assert_eq!(ob["plugin"], "obfs-local");
        assert_eq!(ob["plugin_opts"], "obfs=tls;obfs-host=bing.com");
    }

    #[test]
    fn clash_ss_v2ray_plugin_mapped() {
        let p = yaml_proxy(
            "type: ss\nserver: 1.2.3.4\nport: 8388\ncipher: aes-256-gcm\npassword: pw\nplugin: v2ray-plugin\nplugin-opts:\n  mode: websocket\n  tls: true\n  host: example.com\n  path: /ws\n",
        );
        let ob = clash_yaml_proxy_to_singbox(&p, "X").expect("ss outbound");
        assert_eq!(ob["plugin"], "v2ray-plugin");
        let opts = ob["plugin_opts"].as_str().unwrap();
        assert!(opts.contains("mode=websocket"), "opts: {opts}");
        assert!(opts.contains("tls"), "opts: {opts}");
        assert!(opts.contains("host=example.com"), "opts: {opts}");
        assert!(opts.contains("path=/ws"), "opts: {opts}");
    }

    #[test]
    fn clash_ss_without_plugin_has_no_plugin_field() {
        let p = yaml_proxy(
            "type: ss\nserver: 1.2.3.4\nport: 8388\ncipher: aes-128-gcm\npassword: pw\n",
        );
        let ob = clash_yaml_proxy_to_singbox(&p, "X").expect("ss outbound");
        assert!(ob.get("plugin").is_none(), "plain SS must not carry a plugin");
    }

    #[test]
    fn build_tun_inbound_ipv6_gated() {
        let mut cfg = crate::types::AppConfig::default();
        cfg.enable_ipv6 = false;
        let v4 = build_tun_inbound(&cfg);
        assert_eq!(v4["address"].as_array().unwrap().len(), 1, "IPv4-only address");
        assert_eq!(v4["mtu"], 9000);

        cfg.enable_ipv6 = true;
        let dual = build_tun_inbound(&cfg);
        assert_eq!(dual["address"].as_array().unwrap().len(), 2, "dual-stack address");
    }
}
