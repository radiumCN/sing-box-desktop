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
            if text.contains("vmess://") || text.contains("vless://") || text.contains("ss://") {
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
        SubType::Unknown => Err(anyhow!("???????")),
    }
}

fn parse_clash(content: &str, sub_id: &str) -> Result<(Vec<ProxyNode>, Vec<Value>)> {
    let yaml: YamlValue = serde_yaml::from_str(content)
        .map_err(|e| anyhow!("Clash YAML ????: {}", e))?;

    let proxies = yaml["proxies"]
        .as_sequence()
        .ok_or_else(|| anyhow!("????proxies ??"))?;

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
            group: "??".to_string(),
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

fn clash_yaml_proxy_to_singbox(proxy: &YamlValue, tag: &str) -> Option<Value> {
    let proto = proxy["type"].as_str()?;
    let server = proxy["server"].as_str()?;
    let port = proxy["port"].as_u64()?;

    match proto {
        "ss" => {
            let password = proxy["password"].as_str().unwrap_or("");
            let cipher = proxy["cipher"].as_str().unwrap_or("aes-128-gcm");
            Some(json!({
                "type": "shadowsocks",
                "tag": tag,
                "server": server,
                "server_port": port,
                "method": cipher,
                "password": password
            }))
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
                ob["tls"] = json!({ "enabled": true, "insecure": true });
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
                    tls_obj["insecure"] = json!(true);
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
                "tls": { "enabled": true, "server_name": sni, "insecure": true }
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
                "tls": { "enabled": true, "server_name": sni, "insecure": true }
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
        return Err(anyhow!("????????"));
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
    } else {
        Err(anyhow!("??????: {}", link))
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
        outbound["tls"] = json!({ "enabled": true, "server_name": sni, "insecure": true });
    }

    let node = ProxyNode {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.clone(),
        group: "??".to_string(),
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
            // Plain TLS: allow self-signed certs for compatibility.
            tls["insecure"] = json!(true);
        }
        outbound["tls"] = tls;
    }

    let node = ProxyNode {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.clone(),
        group: "??".to_string(),
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

fn parse_ss(link: &str, sub_id: &str) -> Result<(ProxyNode, Value)> {
    let url = Url::parse(link)?;
    let server = url.host_str().unwrap_or("").to_string();
    let port = url.port().unwrap_or(8388);
    let name = url.fragment().unwrap_or(&server).to_string();

    let user_info = url.username();
    let (method, password) = if let Ok(decoded) = general_purpose::STANDARD.decode(user_info) {
        let s = String::from_utf8(decoded).unwrap_or_default();
        let parts: Vec<&str> = s.splitn(2, ':').collect();
        (
            parts.first().copied().unwrap_or("aes-128-gcm").to_string(),
            parts.get(1).copied().unwrap_or("").to_string(),
        )
    } else {
        let password = url.password().unwrap_or("").to_string();
        (user_info.to_string(), password)
    };

    let outbound = json!({
        "type": "shadowsocks",
        "tag": name,
        "server": server,
        "server_port": port,
        "method": method,
        "password": password
    });

    let node = ProxyNode {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.clone(),
        group: "??".to_string(),
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

    let outbound = json!({
        "type": "trojan",
        "tag": name,
        "server": server,
        "server_port": port,
        "password": password,
        "tls": { "enabled": true, "server_name": sni, "insecure": true }
    });

    let node = ProxyNode {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.clone(),
        group: "??".to_string(),
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
    let insecure = params.get("insecure").map(|v| v == "1").unwrap_or(false);

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
        group: "??".to_string(),
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

fn parse_sip008(content: &str, sub_id: &str) -> Result<(Vec<ProxyNode>, Vec<Value>)> {
    let v: Value = serde_json::from_str(content)?;
    let servers = v["servers"].as_array()
        .ok_or_else(|| anyhow!("SIP008: ????servers ??"))?;

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
            group: "??".to_string(),
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
pub fn build_singbox_config(
    outbounds: &[Value],
    config: &crate::types::AppConfig,
    active_tag: Option<&str>,
) -> Value {
    let selector_outbounds: Vec<Value> = outbounds.iter()
        .map(|ob| Value::String(ob["tag"].as_str().unwrap_or("").to_string()))
        .collect();

    let selected = active_tag
        .unwrap_or_else(|| selector_outbounds.first().and_then(|v| v.as_str()).unwrap_or(""));

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
    all_outbounds.extend_from_slice(&clean_outbounds);

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
    //   2. CN domains            → real IP via the local resolver (direct, fast)
    //   3. EVERYTHING ELSE A/AAAA → fake IP, so the EXIT node performs the real
    //      lookup (correct CDN edge ⇒ full node speed, no per-connection DoT).
    // Non-A/AAAA queries (HTTPS/SVCB/PTR…) fall through to `final: dns_local`.
    //
    // The previous design only handed a fake IP to domains explicitly present in
    // the geosite-geolocation-noncn list, so any foreign domain NOT in that list
    // (e.g. proof.its-paris.scaleway.com) fell back to the China resolver, got a
    // wrong/unreachable IP and could not be opened. A catch-all fake-ip rule
    // ("not CN ⇒ fake IP") removes that whole class of failures.
    let mut dns_rules: Vec<Value> = Vec::new();
    if !server_domains.is_empty() {
        dns_rules.push(json!({ "domain": server_domains, "server": "dns_local" }));
    }
    dns_rules.push(json!({ "rule_set": "geosite-cn", "server": "dns_local" }));
    dns_rules.push(json!({
        "query_type": ["A", "AAAA"],
        "server": "dns_fakeip"
    }));

    let mut cfg = json!({
        "log": { "level": config.log_level, "timestamp": true },
        "dns": {
            "servers": [
                {
                    "type": "udp",
                    "tag": "dns_local",
                    "server": "223.5.5.5"
                },
                {
                    "type": "fakeip",
                    "tag": "dns_fakeip",
                    "inet4_range": "198.18.0.0/15"
                }
            ],
            "rules": dns_rules,
            "final": "dns_local",
            "strategy": "ipv4_only",
            "independent_cache": true
        },
        "inbounds": [
            {
                "type": "mixed",
                "tag": "mixed-in",
                "listen": "127.0.0.1",
                "listen_port": config.mixed_port,
                "set_system_proxy": false
            }
        ],
        "outbounds": all_outbounds,
        "route": {
            "default_domain_resolver": "dns_local",
            "rules": [
                { "action": "sniff" },
                { "protocol": ["dns"], "action": "hijack-dns" },
                { "ip_is_private": true, "outbound": "direct" },
                { "clash_mode": "Direct", "outbound": "direct" },
                { "clash_mode": "Global", "outbound": "proxy" },
                { "rule_set": ["geosite-cn"], "outbound": "direct" },
                { "rule_set": ["geoip-cn"], "outbound": "direct" }
            ],
            "rule_set": [ geosite_cn_rs, geoip_cn_rs ],
            "final": "proxy",
            "auto_detect_interface": true
        },
        "experimental": {
            "clash_api": {
                "external_controller": format!("127.0.0.1:{}", config.api_port),
                "external_ui": "ui",
                "secret": ""
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
        let mut tun_in = json!({
            "type": "tun",
            "tag": "tun-in",
            "address": ["172.19.0.1/30", "fdfe:dcba:9876::1/126"],
            "mtu": 9000,
            "auto_route": true,
            "strict_route": true,
            "stack": "system"
        });

        // On Windows, use a unique interface name per start. If a previous run crashed
        // and left an orphaned adapter behind, a fresh name avoids the WinTun "Cannot
        // create a file when that file already exists" failure entirely. Old
        // "sing-box-tun*" adapters are cleaned up before start by cleanup_stale_tun_adapter().
        //
        // On macOS/Linux the TUN device must be named "utunN"/"tunN" by the kernel, so we
        // omit interface_name and let sing-box pick a valid one automatically.
        #[cfg(target_os = "windows")]
        {
            let unique_suffix = uuid::Uuid::new_v4().simple().to_string();
            let interface_name = format!("sing-box-tun-{}", &unique_suffix[..6]);
            tun_in["interface_name"] = json!(interface_name);
        }

        cfg["inbounds"].as_array_mut().unwrap().push(tun_in);
    }

    cfg
}
