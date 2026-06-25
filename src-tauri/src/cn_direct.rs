/**
 * Single source of truth for the "CN must go direct" lists.
 *
 * Two subsystems independently need to know which mainland-China domains must
 * NOT be tunnelled through the proxy, and they previously hard-coded their own
 * (drifting) copies of the list:
 *   - `subscription::build_singbox_config` — emits DNS + route rules so these
 *     domains resolve to a real IP and route `direct` (so CN-API geo-checks pass).
 *   - `proxy::set_system_proxy` (Windows) — adds them to the WinINet
 *     `ProxyOverride` so WinHTTP-based CN apps open a direct socket instead of
 *     going through sing-box.
 *
 * Both express the SAME intent, so they must stay in sync. Centralising the list
 * here removes the maintenance hazard of editing one copy and forgetting the other.
 *
 * NOTE: the editable default rules in `rules.rs` (阿里系/腾讯系/… presets) are
 * intentionally NOT derived from this list — they are user-facing seed data with
 * per-vendor grouping plus geosite/keyword matchers, a different concern.
 */

/// Canonical CN-core domain suffixes that must always be resolved with a real IP
/// and routed direct, even before `geosite-cn.srs` is available (first launch /
/// missing file). Matched as `domain_suffix`, so e.g. `qq.com` also covers
/// `weixin.qq.com`.
pub const CN_DIRECT_SUFFIXES: &[&str] = &[
    // Tencent / WeChat — screenshot translation, WeChat API, Tencent Cloud OCR
    "qq.com", "wechat.com", "weixin.com", "weixin.qq.com",
    "tencent.com", "tencentcloudapi.com", "qcloud.com",
    "gtimg.cn", "qpic.cn", "myqcloud.com", "tenpay.com",
    // Alibaba
    "taobao.com", "tmall.com", "alicdn.com", "tbcdn.cn",
    "alipay.com", "alibaba.com", "aliyun.com", "aliyuncs.com",
    "amap.com", "autonavi.com", "dingtalk.com",
    // Baidu
    "baidu.com", "bdstatic.com", "bcebos.com",
    // ByteDance
    "bytedance.com", "toutiao.com", "douyin.com",
    "feishu.cn", "feishu.com",
    // Other major CN services
    "bilibili.com", "bilivideo.com", "hdslb.com",
    "weibo.com", "sinaimg.cn", "sina.com",
    "163.com", "126.net", "netease.com",
    "zhihu.com", "zhimg.com",
    "jd.com", "jdcdn.com",
    "meituan.com", "meituan.net",
    "xiaohongshu.com", "pinduoduo.com",
    "iqiyi.com", "youku.com", "sohu.com", "mgtv.com",
    "xiaomi.com", "mi.com", "miui.com",
    "huawei.com", "hicloud.com",
    "12306.cn",
];

/// WeChat desktop process names. In TUN mode `process_name` rules route ALL WeChat
/// traffic direct so screenshot translation / voice / CN-API features keep working
/// regardless of fake-ip DNS or proxy routing. (Ignored for the mixed inbound.)
pub const WECHAT_PROCESSES: &[&str] = &[
    "WeChat.exe", "WeChatApp.exe", "WeChatWeb.exe",
];

/// Build the WinINet `ProxyOverride` fragment for the CN-direct domains:
/// `;*.qq.com;*.wechat.com;…` (leading `;`, no trailing `;`). Appended after the
/// local/private ranges by `proxy::set_system_proxy` in non-global mode.
#[cfg(target_os = "windows")]
pub fn proxy_override_fragment() -> String {
    let mut out = String::new();
    for suffix in CN_DIRECT_SUFFIXES {
        out.push_str(";*.");
        out.push_str(suffix);
    }
    out
}
