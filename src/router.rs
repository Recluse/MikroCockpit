use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::Ipv4Addr,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub host: String,
    pub user: String,
    pub identity: String,
    pub mihomo_url: String,
    pub mihomo_group: String,
    pub route_table: String,
    pub managed_wireguard: Vec<String>,
    pub poll_seconds: u64,
    pub name: String,
    pub lan_cidrs: Vec<String>,
    pub lan_interface_list: String,
    pub wan_interfaces: Vec<String>,
    pub bypass_list: String,
    pub protected_wireguard: Vec<String>,
    pub extra_direct_rules: Vec<std::collections::BTreeMap<String, String>>,
    pub tray_order: Vec<String>,
    pub tray_title: String,
    #[serde(default = "system_style")]
    pub theme: String,
    #[serde(default = "system_style")]
    pub icon_color: String,
    #[serde(default = "crate::status::default_fields")]
    pub status_fields: Vec<String>,
    #[serde(default)]
    pub status_interfaces: Vec<String>,
    #[serde(default)]
    pub mihomo_secret_env: Option<String>,
    #[serde(default)]
    pub mihomo_secret: Option<String>,
}
fn system_style() -> String {
    "system".into()
}
impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let c: Self = serde_json::from_str(&fs::read_to_string(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        c.validate()?;
        Ok(c)
    }
    pub fn validate(&self) -> Result<()> {
        if !["system", "light", "dark"].contains(&self.theme.as_str())
            || ![
                "system",
                "monochrome",
                "blue",
                "purple",
                "green",
                "orange",
                "pink",
            ]
            .contains(&self.icon_color.as_str())
        {
            return Err("Invalid appearance settings".into());
        }
        for s in [
            &self.host,
            &self.user,
            &self.mihomo_group,
            &self.route_table,
        ] {
            if s.is_empty()
                || s.starts_with('-')
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-_:".contains(&b))
            {
                return Err("Invalid name in settings".into());
            }
        }
        // The identity path travels into ssh argv and interface/list names into
        // quote()-wrapped RouterOS scripts (delivered on stdin); keep them
        // printable and never option-like.
        if self.identity.is_empty()
            || self.identity.starts_with('-')
            || !self.identity.bytes().all(|b| (0x20..0x7f).contains(&b))
        {
            return Err("Invalid SSH identity path".into());
        }
        for s in std::iter::once(&self.lan_interface_list).chain(&self.wan_interfaces) {
            if s.is_empty()
                || s.starts_with('-')
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b" .-_".contains(&b))
            {
                return Err("Invalid interface or list name in settings".into());
            }
        }
        if self
            .managed_wireguard
            .iter()
            .any(|s| self.protected_wireguard.contains(s))
        {
            return Err("A protected tunnel cannot be managed".into());
        }
        if !self.mihomo_url.is_empty() {
            let base = self
                .mihomo_url
                .strip_prefix("http://")
                .ok_or("mihomo_url: expected HTTP IP:port within the SSH connection")?;
            let (ip, port) = base.split_once(':').ok_or("mihomo_url: expected IP:port")?;
            let ip: Ipv4Addr = ip.parse().map_err(|_| "mihomo_url: expected IPv4")?;
            if !(ip.is_private() || ip.is_loopback())
                || port.parse::<u16>().ok().filter(|p| *p > 0).is_none()
            {
                return Err("mihomo_url must point to a local IPv4 address and port".into());
            }
        }
        for network in &self.lan_cidrs {
            if cidr(network)? != *network {
                return Err("lan_cidrs must contain canonical IPv4 networks".into());
            }
        }
        if self.lan_cidrs.is_empty()
            || self.bypass_list.is_empty()
            || self.lan_interface_list.is_empty()
        {
            return Err("Configure LAN and interface/exception lists".into());
        }
        for rule in &self.extra_direct_rules {
            if rule.is_empty()
                || rule.keys().any(|k| {
                    ![
                        "src-address",
                        "dst-address",
                        "protocol",
                        "src-port",
                        "dst-port",
                    ]
                    .contains(&k.as_str())
                })
            {
                return Err("Unsupported direct access exception".into());
            }
            for k in ["src-address", "dst-address"] {
                if let Some(a) = rule.get(k) {
                    cidr(a)?;
                }
            }
            if rule
                .get("protocol")
                .is_some_and(|p| !["tcp", "udp", "icmp"].contains(&p.as_str()))
            {
                return Err("Unknown exception protocol".into());
            }
            for k in ["src-port", "dst-port"] {
                if let Some(v) = rule.get(k)
                    && !valid_port_list(v)
                {
                    return Err("Ports must look like 53,80 or 7000-7010".into());
                }
            }
        }
        if self
            .tray_order
            .iter()
            .any(|s| !["direct", "exits", "routes", "wireguard", "bypass"].contains(&s.as_str()))
            || !["exit", "cpu", "name", "icon", "custom"].contains(&self.tray_title.as_str())
        {
            return Err("Invalid tray settings".into());
        }
        if self
            .wan_interfaces
            .iter()
            .any(|w| self.protected_wireguard.contains(w))
        {
            return Err("A protected tunnel must not be a WAN interface".into());
        }
        if !crate::status::valid_fields(&self.status_fields) || self.status_interfaces.len() > 8 {
            return Err("Invalid status fields or too many status interfaces".into());
        }
        if !(5..=300).contains(&self.poll_seconds) {
            return Err("poll_seconds: 5…300".into());
        }
        Ok(())
    }
    pub fn ssh(&self, script: &str) -> Result<String> {
        // The script may embed the mihomo Bearer token, so it travels on stdin —
        // never as an ssh argv element, which any local process can read via ps.
        run_stdin(
            &mut self.ssh_command()?,
            Some(script),
            Duration::from_secs(18),
            false,
        )
    }
    pub(crate) fn ssh_probe(&self, script: &str) -> Result<String> {
        // RouterOS container shell ends the SSH session with status 1 even on success.
        // Only the fixed IP probe uses this; its caller validates the entire stdout.
        run_stdin(&mut self.ssh_command()?, Some(script), Duration::from_secs(18), true)
    }
    fn ssh_command(&self) -> Result<Command> {
        let key = if let Some(s) = self.identity.strip_prefix("~/") {
            format!(
                "{}/{}",
                std::env::var("HOME").map_err(|e| e.to_string())?,
                s
            )
        } else {
            self.identity.clone()
        };
        let mut c = Command::new("/usr/bin/ssh");
        c.args([
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "ConnectTimeout=5",
            "-o",
            "ServerAliveInterval=5",
            "-o",
            "ServerAliveCountMax=1",
            "-o",
            "IdentitiesOnly=yes",
            "-i",
            &key,
            &format!("{}@{}", self.user, self.host),
        ]);
        // No remote command in argv: the RouterOS console executes the script fed
        // on stdin (`sh -s` does not exist on RouterOS — verified against it).
        Ok(c)
    }
    pub fn api(&self, path: &str, body: Option<Value>) -> Result<Value> {
        if self.mihomo_url.is_empty() {
            return Err("Mihomo is not configured in the profile".into());
        }
        let mut s = format!(
            ":put [:serialize to=json value=[/tool fetch url={} output=user as-value duration=8s",
            quote(&format!("{}{}", self.mihomo_url, path))
        );
        let mut headers = String::from("Content-Type: application/json");
        let secret = if let Some(sec) = &self.mihomo_secret {
            Some(sec.clone())
        } else if let Some(env) = &self.mihomo_secret_env {
            Some(std::env::var(env).map_err(|_| format!("Set the {env} environment variable for mihomo"))?)
        } else {
            None
        };
        if let Some(secret) = secret {
            if secret.contains([',', '\r', '\n']) {
                return Err("Invalid mihomo token".into());
            }
            headers.push_str(&format!(",Authorization: Bearer {secret}"));
        }
        s.push_str(&format!(" http-header-field={}", quote(&headers)));
        if let Some(b) = body {
            s.push_str(&format!(
                " http-method=put http-data={}",
                quote(&b.to_string())
            ));
        }
        s.push_str("]]");
        let v: Value =
            serde_json::from_str(self.ssh(&s)?.trim()).map_err(|e| format!("Mihomo fetch: {e}"))?;
        if v["status"] != "finished" {
            return Err(format!("Mihomo fetch did not finish: {}", v["status"]));
        }
        let data = v["data"].as_str().unwrap_or("");
        if data.is_empty() {
            Ok(Value::Null)
        } else {
            serde_json::from_str(data).map_err(|e| e.to_string())
        }
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        let script = format!(
            r#":local resource [/system resource print as-value]; :set ($resource->"uptime") [:tostr [/system resource get uptime]]; :put [:serialize to=json value=$resource]; :put [:serialize to=json value=[/ip route print as-value where routing-table={}]]; :put [:serialize to=json value=[/interface wireguard print as-value proplist=name,disabled,running]]; :put [:serialize to=json value=[/ip firewall address-list print as-value where list={}  and comment="MikroCockpit"]]; :put [:serialize to=json value=[/ip dhcp-server lease print as-value proplist=address,host-name,comment,status,mac-address,dynamic]]; :put [:serialize to=json value=[/ip firewall address-list print as-value where list="mikrocockpit-direct"]]; :put [:serialize to=json value=[/ip firewall filter print as-value]]; :put [:serialize to=json value=[/ip firewall mangle print as-value]]; :put [:serialize to=json value=[/ipv6 firewall filter print as-value]]; :put [:serialize to=json value=[/interface list member print as-value]]; :put [:serialize to=json value=[/interface print stats as-value proplist=name,type,running,disabled,rx-byte,tx-byte]]; :put [:serialize to=json value=[/ip address print as-value proplist=address,interface,actual-interface,disabled,invalid]]"#,
            quote(&self.route_table),
            quote(&self.bypass_list)
        );
        let out = self.ssh(&script)?;
        let mut values = Vec::new();
        for line in out.lines().filter(|l| !l.trim().is_empty()) {
            values.push(serde_json::from_str::<Value>(line.trim()).map_err(|e| {
                format!(
                    "RouterOS JSON: {e}; {}",
                    line.chars().take(160).collect::<String>()
                )
            })?);
        }
        if values.len() != 12 {
            return Err("Incomplete RouterOS response".into());
        }
        let proxies = if self.mihomo_url.is_empty() {
            Ok(json!({"proxies":{}}))
        } else {
            self.api("/proxies", None)
        };
        let (proxies, proxy_error) = match proxies {
            Ok(v) => (v["proxies"].clone(), None),
            Err(e) => (json!({}), Some(e)),
        };
        let wg = rows(&values[2])?;
        let leases = rows(&values[4])?;
        let this_mac = local_ipv4(&self.host).filter(|ip| {
            self.lan_cidrs.iter().any(|n| contains_ip(n, ip))
                && leases.iter().any(|l| text(l, "address") == *ip)
        });
        Ok(Snapshot {
            this_mac,
            internet: None,
            public_ip: None,
            public_ip_source: None,
            public_ip_error: None,
            wan_addresses: rows(&values[11])?
                .into_iter()
                .filter(|a| {
                    !flag(a, "disabled")
                        && !flag(a, "invalid")
                        && (self.wan_interfaces.contains(&text(a, "interface"))
                            || self.wan_interfaces.contains(&text(a, "actual-interface")))
                })
                .map(|a| format!("{} ({})", text(&a, "address"), text(&a, "interface")))
                .collect(),
            wg_addresses: rows(&values[11])?
                .into_iter()
                .filter(|a| {
                    !flag(a, "disabled")
                        && wg.iter().any(|w| text(w, "name") == text(a, "interface"))
                })
                .map(|a| format!("{} ({})", text(&a, "address"), text(&a, "interface")))
                .collect(),
            interfaces: rows(&values[10])?,
            rates: std::collections::BTreeMap::new(),
            resource: values[0].clone(),
            routes: rows(&values[1])?,
            wg,
            bypass: rows(&values[3])?,
            leases,
            direct: rows(&values[5])?,
            filter: rows(&values[6])?,
            mangle: rows(&values[7])?,
            ipv6_filter: rows(&values[8])?,
            members: rows(&values[9])?,
            proxies,
            proxy_error,
        })
    }
    pub fn apply(&self, action: &Action) -> Result<String> {
        // Fresh reads, guarded writes and post-read verification; never retry an ambiguous mutation.
        let before = self.snapshot()?;
        match action {
            Action::Direct(address, _) | Action::EndDirect(address) => {
                if !policy_ready(&before, self) {
                    return Err(
                        "Install and verify the MikroCockpit v1 policy on the router first".into(),
                    );
                }
                let all = address == "all";
                let targets = if all {
                    self.lan_cidrs.clone()
                } else {
                    vec![address.clone()]
                };
                if !all && !self.lan_cidrs.iter().any(|n| contains_ip(n, address)) {
                    return Err("Address is outside the profile LAN".into());
                }
                if matches!(action, Action::Direct(..))
                    && !all
                    && !before.leases.iter().any(|l| text(l, "address") == *address)
                {
                    return Err("Device is not in the current DHCP leases".into());
                }
                let mut script = String::new();
                for target in &targets {
                    script.push_str(&format!("/ip firewall address-list remove [find where list=\"mikrocockpit-direct\" and comment=\"MikroCockpit\" and address={}]; ",quote(target)));
                    if let Action::Direct(_, minutes) = action {
                        if !all
                            && minutes.is_none()
                            && before
                                .leases
                                .iter()
                                .any(|l| text(l, "address") == *address && flag(l, "dynamic"))
                        {
                            return Err("A permanent exception requires a static DHCP lease".into());
                        }
                        script.push_str(&format!("/ip firewall address-list add list=\"mikrocockpit-direct\" comment=\"MikroCockpit\" address={}{}; ",quote(target),direct_timeout(*minutes)?));
                    }
                }
                script.push_str(":put \"OK\"");
                checked(self.ssh(&script)?)?;
                let after = self.snapshot()?;
                for target in targets {
                    let exists = after.direct.iter().any(|r| text(r, "address") == target);
                    if exists != matches!(action, Action::Direct(..)) {
                        return Err("Direct access was not fully applied; refresh status".into());
                    }
                }
            }
            Action::Exit(name) => {
                let g = &before.proxies[&self.mihomo_group];
                if !safe_exit(&before.proxies, name, &mut Vec::new()) {
                    return Err("Exit contains DIRECT, an unknown type or a cycle".into());
                }
                if g["type"] != "Selector"
                    || !g["all"]
                        .as_array()
                        .is_some_and(|a| a.contains(&json!(name)))
                    || ["DIRECT", "REJECT", "PASS"].contains(&name.as_str())
                {
                    return Err("This exit cannot be selected".into());
                }
                self.api(
                    &format!("/proxies/{}", self.mihomo_group),
                    Some(json!({"name": name})),
                )?;
                let after = self.api(&format!("/proxies/{}", self.mihomo_group), None)?;
                if after["now"] != *name {
                    return Err("Mihomo did not confirm the selected exit".into());
                }
            }
            Action::Wireguard(name, enabled) => {
                if self.protected_wireguard.contains(name) || !self.managed_wireguard.contains(name)
                {
                    return Err("This tunnel is protected".into());
                }
                let w = before
                    .wg
                    .iter()
                    .find(|w| text(w, "name") == *name)
                    .ok_or("Interface no longer exists")?;
                if !enabled
                    && before.routes.iter().any(|r| {
                        !flag(r, "disabled") && text(r, "gateway").split(',').any(|g| g == name)
                    })
                {
                    return Err("A route uses this tunnel: change the VPN path first".into());
                }
                let script = format!(
                    ":local i [/interface wireguard find where name={}]; :if ([:len $i] != 1) do={{:error \"interface changed\"}}; :if ([/interface wireguard get $i disabled] != {}) do={{:error \"state changed\"}}; /interface wireguard set $i disabled={}; :put \"OK\"",
                    quote(name),
                    flag(w, "disabled"),
                    !enabled
                );
                checked(self.ssh(&script)?)?;
                let after = self.snapshot()?;
                if !after
                    .wg
                    .iter()
                    .any(|w| text(w, "name") == *name && flag(w, "disabled") != *enabled)
                {
                    return Err("WireGuard state was not confirmed".into());
                }
            }
            Action::Primary(id) => {
                let routes = defaults(&before.routes);
                let target = routes
                    .iter()
                    .find(|r| text(r, ".id") == *id)
                    .ok_or("Route no longer exists")?;
                if flag(target, "disabled") || flag(target, "dynamic") {
                    return Err("Choose an existing enabled static route".into());
                }
                if !policy_ready(&before, self) {
                    return Err("Install the MikroCockpit protection policy first".into());
                }
                let plan = primary_script(
                    &routes,
                    id,
                    &self.route_table,
                    before
                        .wg
                        .iter()
                        .any(|w| text(w, "name") == text(target, "gateway")),
                )?;
                checked(self.ssh(&plan)?)?;
                let after = self.snapshot()?;
                if !after.routes.iter().any(|r| {
                    text(r, ".id") == *id && text(r, "distance") == "1" && flag(r, "active")
                }) {
                    return Err(
                        "Priority changed, but activity was not confirmed — check routes".into(),
                    );
                }
            }
            Action::AddBypass(raw) => {
                let cidr = cidr(raw)?;
                let script = format!(
                    ":if ([:len [/ip firewall address-list find where list={} and address={}]] > 0) do={{:error \"already exists\"}}; /ip firewall address-list add list={} address={} comment=\"MikroCockpit\"; :put \"OK\"",
                    quote(&self.bypass_list),
                    quote(&cidr),
                    quote(&self.bypass_list),
                    quote(&cidr)
                );
                checked(self.ssh(&script)?)?;
                if !self.snapshot()?.bypass.iter().any(|r| {
                    text(r, "address") == cidr
                        || (cidr.ends_with("/32")
                            && text(r, "address") == cidr.trim_end_matches("/32"))
                }) {
                    return Err("Rule was not confirmed".into());
                }
            }
            Action::RemoveBypass(id, address) => {
                if !before.bypass.iter().any(|r| {
                    text(r, ".id") == *id && text(r, "address") == *address && !flag(r, "dynamic")
                }) {
                    return Err("Rule changed or is not owned by MikroCockpit".into());
                }
                let script = format!(
                    ":local i [/ip firewall address-list find where .id={} and list={} and comment=\"MikroCockpit\" and address={}]; :if ([:len $i] != 1) do={{:error \"rule changed\"}}; /ip firewall address-list remove $i; :put \"OK\"",
                    quote(id),
                    quote(&self.bypass_list),
                    quote(address)
                );
                checked(self.ssh(&script)?)?;
                if self
                    .snapshot()?
                    .bypass
                    .iter()
                    .any(|r| text(r, ".id") == *id)
                {
                    return Err("Removal was not confirmed".into());
                }
            }
        }
        Ok("Confirmed by router. New connections use the selected path.".into())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub this_mac: Option<String>,
    pub internet: Option<bool>,
    pub public_ip: Option<String>,
    pub public_ip_source: Option<String>,
    pub public_ip_error: Option<String>,
    pub wan_addresses: Vec<String>,
    pub wg_addresses: Vec<String>,
    pub interfaces: Vec<Value>,
    pub rates: std::collections::BTreeMap<String, (f64, f64)>,
    pub resource: Value,
    pub routes: Vec<Value>,
    pub wg: Vec<Value>,
    pub bypass: Vec<Value>,
    pub leases: Vec<Value>,
    pub direct: Vec<Value>,
    pub filter: Vec<Value>,
    pub mangle: Vec<Value>,
    pub ipv6_filter: Vec<Value>,
    pub members: Vec<Value>,
    pub proxies: Value,
    pub proxy_error: Option<String>,
}
#[derive(Clone, Debug)]
pub enum Action {
    Exit(String),
    Wireguard(String, bool),
    Primary(String),
    AddBypass(String),
    RemoveBypass(String, String),
    Direct(String, Option<u32>),
    EndDirect(String),
}
pub fn text(v: &Value, key: &str) -> String {
    match &v[key] {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        x => x.to_string(),
    }
}
pub fn flag(v: &Value, key: &str) -> bool {
    v[key] == true || v[key] == "true" || v[key] == "yes"
}
pub(crate) fn rows(v: &Value) -> Result<Vec<Value>> {
    if v.is_null() || v == "" {
        Ok(vec![])
    } else {
        v.as_array().cloned().ok_or("Expected a JSON array".into())
    }
}
pub fn defaults(routes: &[Value]) -> Vec<Value> {
    routes
        .iter()
        .filter(|r| text(r, "dst-address") == "0.0.0.0/0")
        .cloned()
        .collect()
}
pub fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for b in s.as_bytes() {
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'$' => out.push_str("\\$"),
            0..=31 | 127..=255 => out.push_str(&format!("\\{b:02X}")),
            b => out.push(*b as char),
        }
    }
    out.push('"');
    out
}
fn direct_timeout(minutes: Option<u32>) -> Result<String> {
    match minutes {
        None => Ok(String::new()),
        Some(15 | 60) => Ok(format!(" timeout={}m", minutes.unwrap())),
        _ => Err("Unsupported duration".into()),
    }
}
/// RouterOS port list: comma-separated `N` or `N-M` items, N in 1..=65535.
pub fn valid_port_list(v: &str) -> bool {
    !v.is_empty()
        && v.split(',').all(|item| {
            let (a, b) = match item.split_once('-') {
                Some((a, b)) => (a, b),
                None => (item, item),
            };
            let parse = |s: &str| s.parse::<u16>().ok().filter(|p| *p > 0);
            match (parse(a), parse(b)) {
                (Some(a), Some(b)) => a <= b,
                _ => false,
            }
        })
}
pub fn cidr(raw: &str) -> Result<String> {
    let s = raw.trim();
    let (ip, prefix) = s.split_once('/').unwrap_or((s, "32"));
    let ip: Ipv4Addr = ip.parse().map_err(|_| "Enter IPv4 or IPv4/prefix")?;
    let n: u32 = prefix.parse().map_err(|_| "Prefix must be 1…32")?;
    if !(1..=32).contains(&n) {
        return Err("Prefix must be 1…32; bypassing all destinations is forbidden".into());
    }
    let mask = u32::MAX << (32 - n);
    let network = Ipv4Addr::from(u32::from(ip) & mask);
    Ok(format!("{network}/{n}"))
}
fn checked(s: String) -> Result<()> {
    if s.trim() == "OK" {
        Ok(())
    } else {
        Err(format!("RouterOS: {}", s.trim()))
    }
}
fn distance(r: &Value) -> Result<u16> {
    text(r, "distance")
        .parse::<u16>()
        .ok()
        .filter(|n| *n <= 255)
        .ok_or("Invalid route distance".into())
}
fn primary_script(routes: &[Value], id: &str, table: &str, wireguard: bool) -> Result<String> {
    if table == "main"
        || routes.is_empty()
        || !routes.iter().any(|r| text(r, ".id") == id)
        || routes.iter().any(|r| {
            flag(r, "dynamic")
                || flag(r, "disabled")
                || flag(r, "blackhole")
                || text(r, "gateway").is_empty()
        })
    {
        return Err("Default routes do not support safe switching".into());
    }
    let mut s = format!(
        ":if ([:len [/ip route find where routing-table={} and dst-address=\"0.0.0.0/0\"]] != {}) do={{:error \"routes changed\"}}; ",
        quote(table),
        routes.len()
    );
    for r in routes {
        let rid = quote(&text(r, ".id"));
        s.push_str(&format!(":if (([/ip route get {rid} routing-table] != {}) or ([/ip route get {rid} dst-address] != \"0.0.0.0/0\") or ([/ip route get {rid} gateway] != {}) or ([/ip route get {rid} distance] != {}) or [/ip route get {rid} disabled]) do={{:error \"route changed\"}}; ",quote(table),quote(&text(r,"gateway")),distance(r)?));
    }
    if wireguard {
        let gateway = quote(&text(
            routes.iter().find(|r| text(r, ".id") == id).unwrap(),
            "gateway",
        ));
        s.push_str(&format!(
            ":local wg [/interface wireguard find where name={gateway} and disabled=no and running=yes]; :if ([:len $wg] != 1) do={{:error \"WireGuard interface is not running\"}}; :local peer [/interface wireguard peers find where interface={gateway} and disabled=no and allowed-address~\"0.0.0.0/0\"]; :if ([:len $peer] != 1) do={{:error \"WireGuard internet peer is missing or ambiguous\"}}; :if (([:len [:tostr [/interface wireguard peers get $peer current-endpoint-address]]] = 0) or ([/interface wireguard peers get $peer last-handshake] >= 2m)) do={{:error \"WireGuard handshake is stale\"}}; "
        ));
    }
    s.push_str(":do { ");
    // RouterOS has no transaction here; restore original distances on script errors.
    for r in routes.iter().filter(|r| text(r, ".id") != id) {
        s.push_str(&format!(
            "/ip route set {} distance=2; ",
            quote(&text(r, ".id"))
        ));
    }
    s.push_str(&format!("/ip route set {} distance=1; ", quote(id)));
    s.push_str(&format!(
        ":delay 2s; :if (![/ip route get {} active]) do={{:error \"target inactive\"}}; ",
        quote(id)
    ));
    s.push_str("} on-error={ ");
    for r in routes {
        s.push_str(&format!(
            "/ip route set {} distance={}; ",
            quote(&text(r, ".id")),
            distance(r)?
        ));
    }
    s.push_str(":error \"switch failed; restored distances\" }; :put \"OK\"");
    Ok(s)
}
pub fn run(c: &mut Command, timeout: Duration) -> Result<String> {
    run_output(c, timeout, false)
}
fn run_output(c: &mut Command, timeout: Duration, probe_stdout: bool) -> Result<String> {
    run_stdin(c, None, timeout, probe_stdout)
}
fn run_stdin(
    c: &mut Command,
    stdin: Option<&str>,
    timeout: Duration,
    probe_stdout: bool,
) -> Result<String> {
    let mut child = c
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdin_pipe = child.stdin.take();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    // Drain both pipes while waiting, so a full stderr cannot deadlock SSH.
    let a = thread::spawn(move || {
        let mut b = Vec::new();
        stdout.read_to_end(&mut b).map(|_| b)
    });
    let b = thread::spawn(move || {
        let mut b = Vec::new();
        stderr.read_to_end(&mut b).map(|_| b)
    });
    // Readers are running already, so a large stdout cannot deadlock this write.
    let mut write_error = None;
    if let (Some(mut pipe), Some(script)) = (stdin_pipe, stdin) {
        // The RouterOS console executes a line only after "\n".
        let mut line = script.as_bytes().to_vec();
        line.push(b'\n');
        if let Err(e) = pipe.write_all(&line) {
            write_error = Some(e.to_string());
        }
    } // pipe dropped here: the remote side sees EOF.
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {}
            Err(e) => {
                // Reap the child before propagating, otherwise it leaks on this path.
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.to_string());
            }
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(30));
    };
    let out = a
        .join()
        .map_err(|_| "stdout reader failed")?
        .map_err(|e| e.to_string())?;
    let err = b
        .join()
        .map_err(|_| "stderr reader failed")?
        .map_err(|e| e.to_string())?;
    let status =
        status.ok_or("Timeout: action result is unknown; refresh status before retrying")?;
    if !(status.success() || probe_stdout && status.code() == Some(1) && err.is_empty()) {
        let detail = String::from_utf8_lossy(&err).trim().to_string();
        return Err(if detail.is_empty() {
            format!("Command failed: {status}")
        } else {
            detail
        });
    }
    if let Some(e) = write_error {
        return Err(format!("ssh stdin write failed: {e}"));
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

pub fn safe_exit(proxies: &Value, name: &str, seen: &mut Vec<String>) -> bool {
    if seen.iter().any(|n| n == name) || seen.len() > 16 {
        return false;
    }
    seen.push(name.into());
    let p = &proxies[name];
    let result = match text(p, "type").as_str() {
        "Vless" | "WireGuard" => true,
        "Selector" | "Fallback" | "URLTest" | "LoadBalance" => {
            p["all"].as_array().is_some_and(|a| {
                !a.is_empty()
                    && a.iter()
                        .all(|v| v.as_str().is_some_and(|n| safe_exit(proxies, n, seen)))
            })
        }
        _ => false,
    };
    seen.pop();
    result
}

// UDP connect selects the local route/address without sending a packet.
fn local_ipv4(host: &str) -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect((host, 22)).ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if !ip.is_unspecified() => Some(ip.to_string()),
        _ => None,
    }
}
pub fn contains_ip(network: &str, address: &str) -> bool {
    let Some((ip, prefix)) = network.split_once('/') else {
        return false;
    };
    let (Ok(ip), Ok(address), Ok(n)) = (
        ip.parse::<Ipv4Addr>(),
        address.parse::<Ipv4Addr>(),
        prefix.parse::<u32>(),
    ) else {
        return false;
    };
    if !(1..=32).contains(&n) {
        return false;
    }
    let mask = u32::MAX << (32 - n);
    u32::from(ip) & mask == u32::from(address) & mask
}
pub fn policy_ready(s: &Snapshot, c: &Config) -> bool {
    crate::policy::ready(s, c)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boundary_and_route_guards() {
        assert_eq!(local_ipv4("127.0.0.1").as_deref(), Some("127.0.0.1"));
        let output = "printf 203.0.113.42; exit 1";
        assert!(
            run(
                Command::new("/bin/sh").args(["-c", output]),
                Duration::from_secs(2)
            )
            .is_err()
        );
        assert_eq!(
            run_output(
                Command::new("/bin/sh").args(["-c", output]),
                Duration::from_secs(2),
                true
            )
            .unwrap(),
            "203.0.113.42"
        );
        assert!(
            run_output(
                Command::new("/bin/sh").args(["-c", "exit 2"]),
                Duration::from_secs(2),
                true
            )
            .is_err()
        );
        assert_eq!(quote("a\"; $x\\\n"), "\"a\\\"; \\$x\\\\\\0A\"");
        assert_eq!(cidr("192.0.2.129/24").unwrap(), "192.0.2.0/24");
        assert!(contains_ip("10.7.0.0/16", "10.7.22.33"));
        assert!(!contains_ip("10.7.0.0/16", "10.8.22.33"));
        assert_eq!(direct_timeout(Some(15)).unwrap(), " timeout=15m");
        assert!(direct_timeout(Some(0)).is_err());
        assert!(direct_timeout(None).unwrap().is_empty());
        for bad in ["0.0.0.0/0", "1.2.3.4/33", "1.2.3.4; /system reboot", "::1"] {
            assert!(cidr(bad).is_err());
        }
        let r = json!({".id":"*1","dst-address":"0.0.0.0/0","distance":1,"gateway":"wg-fin","dynamic":false,"disabled":false});
        assert!(primary_script(std::slice::from_ref(&r), "*1", "main", true).is_err());
        assert!(primary_script(std::slice::from_ref(&r), "*2", "proxy", true).is_err());
        let mut dynamic = r.clone();
        dynamic["dynamic"] = json!(true);
        assert!(primary_script(&[dynamic], "*1", "proxy", true).is_err());
        let fallback = json!({".id":"*2","dst-address":"0.0.0.0/0","distance":2,"gateway":"172.20.0.2","dynamic":false,"disabled":false});
        let p = primary_script(&[r, fallback], "*2", "proxy", false).unwrap();
        assert!(p.contains("routes changed"));
        assert!(p.contains("set \"*1\" distance=2"));
        assert!(p.contains("set \"*2\" distance=1"));
        assert!(p.contains("set \"*1\" distance=1"));
        assert!(p.contains("set \"*2\" distance=2"));
        assert!(p.contains("on-error"));
        let wg = json!({".id":"*3","dst-address":"0.0.0.0/0","distance":2,"gateway":"wg-fin","dynamic":false,"disabled":false});
        let guarded = primary_script(&[json!({".id":"*1","dst-address":"0.0.0.0/0","distance":1,"gateway":"172.20.0.2","dynamic":false,"disabled":false}), wg], "*3", "proxy", true).unwrap();
        assert!(guarded.contains("last-handshake] >= 2m"));
        assert!(guarded.contains("allowed-address~\"0.0.0.0/0\""));
        let proxies = json!({"AUTO":{"type":"Fallback","all":["V"]},"V":{"type":"Vless"}});
        assert!(safe_exit(&proxies, "AUTO", &mut vec![]));
        let mut bad = proxies.clone();
        bad["AUTO"]["all"] = json!(["V", "DIRECT"]);
        assert!(!safe_exit(&bad, "AUTO", &mut vec![]));
        bad["AUTO"]["all"] = json!(["AUTO"]);
        assert!(!safe_exit(&bad, "AUTO", &mut vec![]));
    }
    #[test]
    fn port_lists_and_settings_names_are_validated() {
        assert!(valid_port_list("53"));
        assert!(valid_port_list("3478,19334"));
        assert!(valid_port_list("7777-7778"));
        assert!(valid_port_list("1-65535"));
        for bad in [
            "",
            "0",
            "65536",
            "80,,443",
            "7000-7",
            "7010-7000",
            "80; /system reboot",
            "http",
            "-1",
            "1-2-3",
        ] {
            assert!(!valid_port_list(bad), "must reject {bad:?}");
        }
        // identity: printable, not option-like (spaces are fine — argv, not shell)
        for bad_identity in ["", "-oProxyCommand=x", "ключ\npath", "tab\there"] {
            let mut c = test_config();
            c.identity = bad_identity.into();
            assert!(c.validate().is_err(), "identity {bad_identity:?}");
        }
        let mut ok = test_config();
        ok.identity = "/Users/Some User/.ssh/id_ed25519".into();
        assert!(ok.validate().is_ok());
        // interface / list names: no slashes, quotes or control bytes
        for bad_name in ["", "-LAN", "LAN name!", "LAN\"x", "et/her1"] {
            let mut c = test_config();
            c.lan_interface_list = bad_name.into();
            assert!(c.validate().is_err(), "list {bad_name:?}");
        }
    }
    fn test_config() -> Config {
        serde_json::from_value(serde_json::json!({
            "host": "192.0.2.1", "user": "admin", "identity": "~/.ssh/id_ed25519",
            "mihomo_url": "", "mihomo_group": "PROXY", "route_table": "proxy",
            "managed_wireguard": [], "poll_seconds": 20, "name": "Test",
            "lan_cidrs": ["192.0.2.0/24"], "lan_interface_list": "LAN",
            "wan_interfaces": ["ether1"], "bypass_list": "vpn-bypass",
            "protected_wireguard": [], "extra_direct_rules": [],
            "tray_order": ["direct"], "tray_title": "custom"
        }))
        .expect("test config")
    }
}
