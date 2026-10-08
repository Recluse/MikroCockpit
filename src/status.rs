use crate::router::{Config, Snapshot, flag, text};

pub const FIELDS: &[&str] = &[
    "internet",
    "connection",
    "speed",
    "exit",
    "enabled",
    "interfaces",
    "cpu",
    "uptime",
    "wan_ip",
    "wg_ip",
    "public_ip",
];
pub fn default_fields() -> Vec<String> {
    ["internet", "speed", "exit"]
        .into_iter()
        .map(String::from)
        .collect()
}
pub fn valid_fields(fields: &[String]) -> bool {
    fields.len() <= FIELDS.len() && fields.iter().all(|f| FIELDS.contains(&f.as_str()))
}
fn counter(v: &serde_json::Value, key: &str) -> Option<u64> {
    text(v, key).parse().ok()
}
pub fn rates(next: &mut Snapshot, old: &Snapshot, seconds: f64) {
    if seconds <= 0.0 {
        return;
    }
    for i in &next.interfaces {
        let name = text(i, "name");
        let Some(before) = old.interfaces.iter().find(|v| text(v, "name") == name) else {
            continue;
        };
        let delta = |key| {
            counter(i, key)?
                .checked_sub(counter(before, key)?)
                .map(|n| n as f64 * 8.0 / seconds)
        };
        if let (Some(rx), Some(tx)) = (delta("rx-byte"), delta("tx-byte")) {
            next.rates.insert(name, (rx, tx));
        }
    }
}
fn rate(n: f64) -> String {
    if n >= 1_000_000.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else {
        format!("{:.0}k", n / 1000.0)
    }
}
pub fn exit(snapshot: &Snapshot, config: &Config) -> String {
    let route = snapshot.routes.iter().find(|r| {
        text(r, "dst-address") == "0.0.0.0/0" && flag(r, "active") && !flag(r, "disabled")
    });
    let gateway = route.map(|r| text(r, "gateway")).unwrap_or_default();
    if snapshot.wg.iter().any(|w| text(w, "name") == gateway) {
        return route
            .map(|r| text(r, "comment"))
            .filter(|name| !name.is_empty())
            .unwrap_or(gateway);
    }
    let selected = text(&snapshot.proxies[&config.mihomo_group], "now");
    let actual = text(&snapshot.proxies[&selected], "now");
    if !actual.is_empty() {
        actual
    } else if !selected.is_empty() {
        selected
    } else {
        route
            .map(|r| text(r, "comment"))
            .filter(|name| !name.is_empty())
            .unwrap_or(gateway)
    }
}
pub fn title(snapshot: Option<&Snapshot>, error: Option<&str>, config: &Config) -> String {
    if error.is_some() {
        return "Router unavailable".into();
    }
    let Some(s) = snapshot else {
        return "Connecting…".into();
    };
    let names = if config.status_interfaces.is_empty() {
        &config.wan_interfaces
    } else {
        &config.status_interfaces
    };
    let mut parts = Vec::new();
    for field in &config.status_fields {
        let value = match field.as_str() {
            "internet" => match s.internet {
                Some(true) => "NET UP".into(),
                Some(false) => "NET DOWN".into(),
                None => "NET …".into(),
            },
            "connection" => {
                if s.interfaces.iter().any(|i| {
                    names.contains(&text(i, "name")) && flag(i, "running") && !flag(i, "disabled")
                }) {
                    "LINK UP".into()
                } else {
                    "LINK DOWN".into()
                }
            }
            "speed" => {
                let values: Vec<_> = names.iter().filter_map(|n| s.rates.get(n)).collect();
                if values.len() != names.len() || values.is_empty() {
                    "↓… ↑…".into()
                } else {
                    format!(
                        "↓{} ↑{} b/s",
                        rate(values.iter().map(|v| v.0).sum()),
                        rate(values.iter().map(|v| v.1).sum())
                    )
                }
            }
            "exit" => exit(s, config),
            "enabled" => {
                let mut on: Vec<_> =
                    s.wg.iter()
                        .filter(|w| !flag(w, "disabled"))
                        .map(|w| text(w, "name"))
                        .collect();
                if !config.mihomo_url.is_empty() && s.proxy_error.is_none() {
                    on.push("proxy".into());
                }
                on.join("+")
            }
            "interfaces" => names
                .iter()
                .map(|n| {
                    let i = s.interfaces.iter().find(|v| text(v, "name") == *n);
                    format!(
                        "{}{}",
                        if i.is_some_and(|v| flag(v, "running") && !flag(v, "disabled")) {
                            "🟢"
                        } else {
                            "🔴"
                        },
                        n
                    )
                })
                .collect::<Vec<_>>()
                .join(" "),
            "wg_ip" => format!("WG {}", wg_ip(s)),
            "wan_ip" => format!("WAN {}", wan_ip(s)),
            "public_ip" => format!("IP {}", s.public_ip.as_deref().unwrap_or("…")),
            "cpu" => format!("CPU {}%", text(&s.resource, "cpu-load")),
            "uptime" => format!("up {}", text(&s.resource, "uptime")),
            _ => String::new(),
        };
        if !value.is_empty() {
            parts.push(value);
        }
    }
    parts.join(" · ")
}
pub fn wg_ip(s: &Snapshot) -> String {
    if s.wg_addresses.is_empty() {
        "unavailable".into()
    } else {
        s.wg_addresses.join(", ")
    }
}
pub fn wan_ip(s: &Snapshot) -> String {
    if s.wan_addresses.is_empty() {
        "unavailable".into()
    } else {
        s.wan_addresses.join(", ")
    }
}
// Reachability is measured from this Mac, not inferred from tunnel/link flags.
pub fn probe() -> bool {
    for url in [
        "https://cp.cloudflare.com/generate_204",
        "https://www.gstatic.com/generate_204",
    ] {
        let result = crate::router::run(
            std::process::Command::new("/usr/bin/curl").args([
                "--silent",
                "--noproxy",
                "*",
                "--connect-timeout",
                "2",
                "--max-time",
                "3",
                "--output",
                "/dev/null",
                "--write-out",
                "%{http_code}",
                url,
            ]),
            std::time::Duration::from_secs(4),
        );
        if result.is_ok_and(|code| code == "204") {
            return true;
        }
    }
    false
}
pub fn icon_state(s: Option<&Snapshot>, error: bool, c: &Config) -> &'static str {
    let Some(s) = s.filter(|_| !error) else {
        return "unknown";
    };
    let Some(online) = s.internet else {
        return "unknown";
    };
    // Purple means reachable + selected VPN default, not an egress identity test.
    // Direct exceptions can apply to individual devices/destinations.
    let all_direct = c
        .lan_cidrs
        .iter()
        .all(|n| s.direct.iter().any(|r| text(r, "address") == *n));
    let vpn = !all_direct
        && s.routes.iter().any(|r| {
            text(r, "dst-address") == "0.0.0.0/0"
                && flag(r, "active")
                && !flag(r, "disabled")
                && !c.wan_interfaces.contains(&text(r, "gateway"))
        });
    if !online {
        "offline"
    } else if vpn {
        "vpn"
    } else {
        "online"
    }
}
pub fn icon(s: Option<&Snapshot>, error: bool, c: &Config) -> tray_icon::Icon {
    let kind = icon_state(s, error, c);
    if kind == "unknown" {
        return unknown_icon();
    }
    let bytes: &[u8] = if kind == "offline" {
        include_bytes!("../assets/tray/offline@2x.png")
    } else if kind == "vpn" {
        include_bytes!("../assets/tray/vpn@2x.png")
    } else {
        include_bytes!("../assets/tray/online@2x.png")
    };
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().expect("bundled PNG");
    let mut pixels = vec![0; reader.output_buffer_size().expect("small PNG")];
    let frame = reader.next_frame(&mut pixels).expect("bundled PNG frame");
    assert_eq!(frame.color_type, png::ColorType::Rgba);
    pixels.truncate(frame.buffer_size());
    tray_icon::Icon::from_rgba(pixels, frame.width, frame.height).expect("RGBA icon")
}

fn unknown_icon() -> tray_icon::Icon {
    let mut rgba = vec![0; 22 * 22 * 4];
    for y in 4..18 {
        for x in 4..18 {
            if (x == y || x + y == 21) || y == 17 {
                let i = (y * 22 + x) * 4;
                rgba[i..i + 4].copy_from_slice(&[150, 150, 150, 255]);
            }
        }
    }
    tray_icon::Icon::from_rgba(rgba, 22, 22).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_counter_rates_and_reset() {
        let old = crate::demo();
        let mut next = old.clone();
        next.interfaces[0]["rx-byte"] = serde_json::json!(2000);
        rates(&mut next, &old, 2.0);
        assert_eq!(next.rates["ether1"], (4000.0, 0.0));
        next.rates.clear();
        next.interfaces[0]["rx-byte"] = serde_json::json!(1);
        rates(&mut next, &old, 2.0);
        assert!(next.rates.is_empty());
        let mut c: Config = serde_json::from_str(include_str!("../settings.example.json")).unwrap();
        c.status_fields = vec!["interfaces".into(), "speed".into()];
        assert!(title(Some(&old), None, &c).contains("🟢ether1"));
        assert!(title(Some(&old), None, &c).contains("↓… ↑…"));
        assert_eq!(exit(&old, &c), "AMS-grpc");
        let mut wg = old.clone();
        wg.routes[0]["active"] = serde_json::json!(false);
        wg.routes[1]["active"] = serde_json::json!(true);
        assert_eq!(exit(&wg, &c), "WireGuard · Finland");
    }
}
