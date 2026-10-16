mod egress;
mod locale;
mod menu_refresh;
mod native;
mod policy;
mod router;
mod status;
mod web;
use router::{Action, Config, Result, Snapshot, defaults, flag, text};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tao::{
    event::{Event, StartCause},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::macos::{ActivationPolicy, EventLoopExtMacOS},
};
use tray_icon::{
    TrayIcon, TrayIconBuilder,
    menu::{ContextMenu, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
};

#[derive(Clone)]
enum Request {
    Refresh,
    Change(Action),
    AddBypass,
}
enum AppEvent {
    Menu(MenuEvent),
    Web(Request),
    Preferences(serde_json::Value),
    NativePreferences(serde_json::Value),
    Updated(Box<Result<Snapshot>>, Option<String>, bool),
}

fn config_path() -> PathBuf {
    if let Some(p) = std::env::var_os("MIKROCOCKPIT_CONFIG") {
        return p.into();
    }
    if let Ok(exe) = std::env::current_exe() {
        let p = exe.parent().unwrap().join("../../../settings.json");
        if p.exists() {
            return p;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("settings.json")
}
fn dialog(message: &str, input: bool) -> Result<Option<String>> {
    let code = if input {
        r#"on run argv
try
set r to display dialog (item 1 of argv) with title "MikroCockpit" default answer "" buttons {"Cancel", "Add"} default button "Add" cancel button "Cancel"
return text returned of r
on error number -128
return "__CANCEL__"
end try
end run"#
    } else {
        r#"on run argv
try
display dialog (item 1 of argv) with title "MikroCockpit" buttons {"Cancel", "Apply"} default button "Cancel" cancel button "Cancel"
return "OK"
on error number -128
return "__CANCEL__"
end try
end run"#
    };
    let s = router::run(
        Command::new("/usr/bin/osascript").args(["-e", code, message]),
        Duration::from_secs(180),
    )?;
    if s.trim() == "__CANCEL__" {
        Ok(None)
    } else {
        Ok(Some(s.trim().into()))
    }
}
fn perform(config: &Config, request: Request, audit_path: &std::path::Path) -> Result<String> {
    let action = match request {
        Request::Refresh => return Ok(String::new()),
        Request::AddBypass => match dialog(
            "Destination IPv4 address or subnet to access directly (for example 203.0.113.8/32). Applies to the entire LAN and persists after reboot.",
            true,
        )? {
            Some(s) => Action::AddBypass(s),
            None => return Ok("Cancelled".into()),
        },
        Request::Change(a) => a,
    };
    let prompt=match &action {
        Action::Primary(_)=>Some("Change the primary VPN default route for the entire LAN? Connections may be interrupted. The kill switch stays enabled.".into()),
        Action::Wireguard(n,on)=>Some(format!("{} tunnel {n}?",if *on {"Enable"} else {"Disable"})),
        Action::Direct(a,minutes)=>Some(format!("Allow {} direct internet access, bypassing the router VPN? Duration: {}. This is an explicit VPN exception; existing connections may need to be reopened.",if a=="all"{"THE ENTIRE LAN"}else{a},minutes.map(|m|format!("{m} minutes, then automatic return")).unwrap_or("PERMANENT, until manually removed".into()))),
        Action::EndDirect(a)=>Some(format!("Remove direct access for {a}? Other explicit exceptions will still apply.")),
        Action::RemoveBypass(_,a)=>Some(format!("Remove exception {a}? New connections to this network will use the VPN.")),
        _=>None,
    };
    if let Some(p) = prompt
        && dialog(&p, false)?.is_none()
    {
        return Ok("Cancelled".into());
    }
    audit(audit_path, &format!("apply: {action:?}"));
    config.apply(&action)
}
fn route_name(route: &serde_json::Value) -> String {
    let comment = text(route, "comment");
    if comment.is_empty() {
        text(route, "gateway")
    } else {
        comment
    }
}
fn audit(path: &std::path::Path, text: &str) {
    let line = serde_json::json!({"unix_time":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),"result":text});
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut f) => {
            if let Err(e) = writeln!(f, "{line}") {
                eprintln!("MikroCockpit: audit write failed: {e}");
            }
        }
        Err(e) => {
            // The audit trail is an advertised feature; fail loudly, not silently.
            eprintln!("MikroCockpit: audit open failed ({}): {e}", path.display());
        }
    }
}
fn label(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(130).collect()
}
fn item(menu: &Menu, title: &str, enabled: bool) -> MenuItem {
    let i = MenuItem::new(label(title), enabled, None);
    menu.append(&i).expect("native menu");
    i
}
fn subitem(menu: &Submenu, title: &str, enabled: bool) -> MenuItem {
    let i = MenuItem::new(label(title), enabled, None);
    menu.append(&i).expect("native submenu");
    i
}
fn icon() -> tray_icon::Icon {
    let mut rgba = vec![0; 22 * 22 * 4];
    for y in 0..22 {
        for x in 0..22 {
            let on = (5..17).contains(&y)
                && (x == 3 || x == 4 || x == 17 || x == 18 || x == y - 2 || x == 23 - y);
            if on {
                rgba[(y * 22 + x) * 4 + 3] = 255;
            }
        }
    }
    tray_icon::Icon::from_rgba(rgba, 22, 22).unwrap()
}
fn build_menu(
    snapshot: Option<&Snapshot>,
    error: Option<&str>,
    status: &str,
    busy: bool,
    config: &Config,
) -> (Menu, HashMap<String, Request>, String, String) {
    let menu = Menu::new();
    let mut sections = HashMap::new();
    let mut actions = HashMap::new();
    let mut title = "MC · …".to_string();
    if let Some(s) = snapshot {
        let used: Vec<_> = defaults(&s.routes)
            .into_iter()
            .filter(|r| flag(r, "active") && !flag(r, "disabled"))
            .collect();
        let paths = used
            .iter()
            .map(|r| text(r, "gateway"))
            .collect::<Vec<_>>()
            .join(" + ");
        let group = &s.proxies[&config.mihomo_group];
        let selected = text(group, "now");
        let actual = if s.proxies[&selected]["now"].is_string() {
            text(&s.proxies[&selected], "now")
        } else {
            selected.clone()
        };
        let active_exit = status::exit(s, config);
        title = if error.is_some() {
            "MC · offline".into()
        } else if s.proxy_error.is_some() {
            "MC · API !".into()
        } else {
            format!(
                "MC · {}",
                if active_exit.is_empty() {
                    "VPN"
                } else {
                    &active_exit
                }
            )
        };
        item(&menu, "Router", false);
        item(&menu, &format!("MikroCockpit · {}", config.host), false);
        item(
            &menu,
            &format!(
                "RouterOS {} · CPU {}% · RAM {} MB free",
                text(&s.resource, "version"),
                text(&s.resource, "cpu-load"),
                s.resource["free-memory"].as_u64().unwrap_or(0) / 1048576
            ),
            false,
        );
        item(
            &menu,
            &format!(
                "Route {} → {}",
                config.route_table,
                if paths.is_empty() {
                    "NO ACTIVE DEFAULT"
                } else {
                    &paths
                }
            ),
            false,
        );
        menu.append(&PredefinedMenuItem::separator()).unwrap();
        item(&menu, "Addresses", false);
        item(&menu, &format!("WAN IP · {}", status::wan_ip(s)), false);
        item(
            &menu,
            &format!(
                "VPN public IP · {}",
                s.public_ip.as_deref().unwrap_or("unavailable")
            ),
            false,
        );
        if let Some(detail) = s.public_ip_source.as_ref().or(s.public_ip_error.as_ref()) {
            item(&menu, &format!("IP probe · {detail}"), false);
        }
        item(&menu, &format!("WG local IP · {}", status::wg_ip(s)), false);
        menu.append(&PredefinedMenuItem::separator()).unwrap();
        item(&menu, "Connection", false);
        let ready = router::policy_ready(s, config);
        item(
            &menu,
            if ready {
                "Direct access protection: policy verified"
            } else {
                "MikroCockpit protection: not configured yet"
            },
            false,
        );
        if !s.direct.is_empty() {
            let all = config
                .lan_cidrs
                .iter()
                .all(|n| s.direct.iter().any(|r| text(r, "address") == *n));
            if error.is_none() {
                title = if all {
                    "MC · VPN OFF".into()
                } else {
                    format!("MC · direct {}", s.direct.len())
                };
            }
        }
        item(
            &menu,
            "Direct access switch",
            ready && !busy && error.is_none(),
        );
        let entire_lan_direct = config
            .lan_cidrs
            .iter()
            .all(|n| s.direct.iter().any(|r| text(r, "address") == *n));
        item(
            &menu,
            "This Mac access switch",
            ready && !busy && error.is_none() && s.this_mac.is_some() && !entire_lan_direct,
        );
        menu.append(&PredefinedMenuItem::separator()).unwrap();
        let modes = Submenu::new("Direct internet / testing", true);
        if !ready {
            subitem(&modes, "Install the policy on the router first", false);
        }
        let all = Submenu::new("Entire LAN — VPN OFF", true);
        for (caption, minutes) in [
            ("For 15 minutes", Some(15)),
            ("For 1 hour", Some(60)),
            ("Permanently…", None),
        ] {
            let i = subitem(&all, caption, ready && !busy && error.is_none());
            actions.insert(
                i.id().0.clone(),
                Request::Change(Action::Direct("all".into(), minutes)),
            );
        }
        modes.append(&all).unwrap();
        for lease in &s.leases {
            let address = text(lease, "address");
            if !config
                .lan_cidrs
                .iter()
                .any(|n| router::contains_ip(n, &address))
            {
                continue;
            }
            let name = if text(lease, "host-name").is_empty() {
                text(lease, "comment")
            } else {
                text(lease, "host-name")
            };
            let device = Submenu::new(label(&format!("{name} · {address}")), true);
            for (caption, minutes) in [
                ("Direct for 15 minutes", Some(15)),
                ("Direct for 1 hour", Some(60)),
                ("Direct permanently…", None),
            ] {
                let i = subitem(&device, caption, ready && !busy && error.is_none());
                actions.insert(
                    i.id().0.clone(),
                    Request::Change(Action::Direct(address.clone(), minutes)),
                );
            }
            modes.append(&device).unwrap();
        }
        for r in &s.direct {
            let address = text(r, "address");
            let ttl = text(r, "timeout");
            let i = subitem(
                &modes,
                &format!(
                    "Return to VPN: {address} · {}",
                    if ttl.is_empty() { "permanent" } else { &ttl }
                ),
                ready && !busy && error.is_none(),
            );
            actions.insert(
                i.id().0.clone(),
                Request::Change(Action::EndDirect(address)),
            );
        }
        subitem(&modes, "IPv6 blocked. Timer runs on the router.", false);
        sections.insert("direct", modes);
        let exits = Submenu::new(format!("VLESS exit · {actual}"), true);
        if let Some(names) = group["all"].as_array() {
            for n in names
                .iter()
                .filter_map(|v| v.as_str())
                .filter(|n| !["DIRECT", "REJECT", "PASS"].contains(n))
            {
                let i = subitem(
                    &exits,
                    &format!("{} {n}", if n == selected { "✓" } else { "  " }),
                    !busy && error.is_none() && router::safe_exit(&s.proxies, n, &mut vec![]),
                );
                actions.insert(i.id().0.clone(), Request::Change(Action::Exit(n.into())));
            }
        }
        if let Some(e) = &s.proxy_error {
            subitem(&exits, &format!("Unavailable: {e}"), false);
        }
        sections.insert("exits", exits);
        let active_path = defaults(&s.routes)
            .iter()
            .find(|r| flag(r, "active") && text(r, "distance") == "1")
            .map(route_name)
            .unwrap_or_else(|| "no active path".into());
        let routes = Submenu::new(format!("Internet VPN · {active_path}"), true);
        for r in defaults(&s.routes) {
            let id = text(&r, ".id");
            let enabled =
                !busy && error.is_none() && !flag(&r, "disabled") && !flag(&r, "dynamic") && ready;
            let i = subitem(
                &routes,
                &format!(
                    "{} {} · distance {}{}",
                    if flag(&r, "active") && text(&r, "distance") == "1" {
                        "✓"
                    } else {
                        "  "
                    },
                    route_name(&r),
                    text(&r, "distance"),
                    if flag(&r, "disabled") {
                        " · disabled"
                    } else {
                        ""
                    }
                ),
                enabled,
            );
            actions.insert(i.id().0.clone(), Request::Change(Action::Primary(id)));
        }
        subitem(
            &routes,
            if ready {
                "Kill switch verified · no direct WAN fallback"
            } else {
                "Kill switch not verified · switching disabled"
            },
            false,
        );
        sections.insert("routes", routes);
        let wg = Submenu::new("WireGuard", true);
        for w in &s.wg {
            let name = text(w, "name");
            let on = !flag(w, "disabled");
            let enabled = !busy
                && error.is_none()
                && config.managed_wireguard.contains(&name)
                && !config.protected_wireguard.contains(&name);
            let i = subitem(
                &wg,
                &format!(
                    "{} {}{}",
                    if on { "✓" } else { "  " },
                    name,
                    if config.protected_wireguard.contains(&name) {
                        " · protected"
                    } else {
                        ""
                    }
                ),
                enabled,
            );
            actions.insert(
                i.id().0.clone(),
                Request::Change(Action::Wireguard(name.clone(), !on)),
            );
            if enabled && !s.routes.iter().any(|r| text(r, "gateway") == name) {
                subitem(&wg, "Tunnel exists; VPN route is not configured yet", false);
            }
        }
        sections.insert("wireguard", wg);
        let bypass = Submenu::new("Exceptions: direct access", true);
        let i = subitem(&bypass, "Add IPv4 / subnet…", !busy && error.is_none());
        actions.insert(i.id().0.clone(), Request::AddBypass);
        for r in &s.bypass {
            let address = text(r, "address");
            let i = subitem(
                &bypass,
                &format!("Remove {address}…"),
                !busy && error.is_none(),
            );
            actions.insert(
                i.id().0.clone(),
                Request::Change(Action::RemoveBypass(text(r, ".id"), address)),
            );
        }
        subitem(
            &bypass,
            "Only exceptions created by MikroCockpit are shown",
            false,
        );
        sections.insert("bypass", bypass);
        for key in &config.tray_order {
            if let Some(section) = sections.get(key.as_str()) {
                menu.append(section).unwrap();
            }
        }
        if error.is_none() && s.direct.is_empty() {
            match config.tray_title.as_str() {
                "cpu" => title = format!("MC · {}%", text(&s.resource, "cpu-load")),
                "name" => title = format!("MC · {}", config.name),
                "icon" => title = String::new(),
                _ => {}
            }
        }
    } else {
        item(&menu, "MikroCockpit · connecting to router…", false);
    }
    if let Some(e) = error {
        title = "MC · offline".into();
        item(&menu, &format!("Error: {e}"), false);
    }
    item(&menu, status, false);
    menu.append(&PredefinedMenuItem::separator()).unwrap();
    menu.append(&MenuItem::with_id(
        "dashboard",
        "Open dashboard…",
        true,
        None,
    ))
    .unwrap();
    menu.append(&MenuItem::with_id(
        "web-dashboard",
        "Open in browser…",
        true,
        None,
    ))
    .unwrap();
    let refresh = item(&menu, "Refresh now", !busy);
    actions.insert(refresh.id().0.clone(), Request::Refresh);
    menu.append(&MenuItem::with_id("settings", "Open settings…", true, None))
        .unwrap();
    menu.append(&MenuItem::with_id("audit", "Activity log…", true, None))
        .unwrap();
    menu.append(&MenuItem::with_id(
        "report",
        "Open latest snapshot…",
        true,
        None,
    ))
    .unwrap();
    menu.append(&MenuItem::with_id("quit", "Quit", true, None))
        .unwrap();
    let tooltip = if let Some(e) = error {
        format!("MikroCockpit: {e}")
    } else {
        format!("MikroCockpit · {status}")
    };
    if config.tray_title == "custom" {
        title = status::title(snapshot, error, config);
    }
    let all_direct = snapshot.is_some_and(|s| {
        config
            .lan_cidrs
            .iter()
            .all(|n| s.direct.iter().any(|r| text(r, "address") == *n))
    });
    native::style_menu(
        menu.ns_menu(),
        &config.theme,
        &config.icon_color,
        all_direct,
        snapshot.and_then(|s| s.this_mac.as_deref()),
        snapshot.is_some_and(|s| {
            s.this_mac.as_ref().is_some_and(|ip| {
                s.direct
                    .iter()
                    .any(|r| text(r, "address").trim_end_matches("/32") == ip)
            })
        }),
    );
    (menu, actions, title, tooltip)
}
fn demo() -> Snapshot {
    Snapshot {
        this_mac: Some("192.168.88.10".into()),
        internet: Some(true),
        public_ip: Some("203.0.113.42".into()),
        public_ip_source: Some("Router container VPN (demo)".into()),
        public_ip_error: None,
        wan_addresses: vec!["198.51.100.10/24 (ether1)".into()],
        wg_addresses: vec!["10.10.0.2/24 (wg-fin)".into()],
        interfaces: vec![
            serde_json::json!({"name":"ether1","running":true,"rx-byte":1000,"tx-byte":500}),
        ],
        rates: std::collections::BTreeMap::new(),
        resource: serde_json::json!({"version":"demo","cpu-load":12,"free-memory":83886080,"uptime":"2d 03:12:09"}),
        routes: vec![
            serde_json::json!({".id":"*1","dst-address":"0.0.0.0/0","gateway":"172.20.0.2","distance":1,"active":true,"comment":"VLESS / Mihomo"}),
            serde_json::json!({".id":"*2","dst-address":"0.0.0.0/0","gateway":"wg-fin","distance":2,"active":false,"comment":"WireGuard · Finland"}),
        ],
        wg: vec![
            serde_json::json!({"name":"wg-fin","disabled":false}),
            serde_json::json!({"name":"wg-colo","disabled":false}),
        ],
        bypass: vec![],
        leases: vec![
            serde_json::json!({"address":"192.168.88.10","host-name":"MacBook","dynamic":false}),
            serde_json::json!({"address":"192.168.88.11","host-name":"Phone","dynamic":true}),
        ],
        direct: vec![],
        filter: vec![],
        mangle: vec![],
        ipv6_filter: vec![],
        members: vec![],
        proxies: serde_json::json!({"PROXY":{"type":"Selector","now":"AUTO","all":["AUTO","AMS-grpc","FRA-grpc"]},"AUTO":{"now":"AMS-grpc"}}),
        proxy_error: None,
    }
}
fn apply_preferences(config: &mut Config, v: &serde_json::Value) {
    if let Some(theme) = v["theme"].as_str() {
        config.theme = theme.into();
    }
    if let Some(color) = v["icon_color"].as_str() {
        config.icon_color = color.into();
    }
    if let Some(a) = v["tray_order"].as_array() {
        let keys: Vec<_> = a
            .iter()
            .filter_map(|s| s.as_str())
            .filter(|s| ["direct", "exits", "routes", "wireguard", "bypass"].contains(s))
            .map(str::to_string)
            .collect();
        config.tray_order = keys;
    }
    if let Some(fields) = v["status_fields"].as_array() {
        let fields: Vec<String> = fields
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        if status::valid_fields(&fields) {
            config.status_fields = fields;
        }
    }
    if let Some(names) = v["status_interfaces"].as_array() {
        config.status_interfaces = names
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .take(8)
            .collect();
    }
    if let Some(t) = v["tray_title"].as_str()
        && ["exit", "cpu", "name", "icon", "custom"].contains(&t)
    {
        config.tray_title = t.into();
    }
}
fn main() {
    if let Err(e) = start() {
        eprintln!("MikroCockpit: {e}");
        if !std::env::args().any(|a| a.starts_with("--")) {
            let _=router::run(Command::new("/usr/bin/osascript").args(["-e", "on run argv\ndisplay alert \"MikroCockpit\" message (item 1 of argv) as critical\nend run", &e]),Duration::from_secs(120));
        }
        std::process::exit(1)
    }
}
fn start() -> Result<()> {
    let path = config_path();
    let mut config = Config::load(&path)?;
    let args: Vec<_> = std::env::args().collect();
    if args.iter().any(|a| a == "--policy-plan") {
        print!("{}", policy::setup(&config)?);
        return Ok(());
    }
    if args.iter().any(|a| a == "--check") {
        println!(
            "{}",
            serde_json::to_string_pretty(&config.snapshot()?).unwrap()
        );
        return Ok(());
    }
    let memory_smoke = args.iter().any(|a| a == "--memory-smoke");
    let smoke_duration = Duration::from_secs(if memory_smoke { 20 } else { 3 });
    let demo_mode = memory_smoke
        || args
            .iter()
            .any(|a| a == "--demo" || a == "--smoke" || a == "--native-smoke");
    let _ = locale::current();
    let smoke = memory_smoke || args.iter().any(|a| a == "--smoke" || a == "--native-smoke");
    let native_smoke = args.iter().any(|a| a == "--native-smoke");
    let mut show_dashboard = native_smoke || args.iter().any(|a| a == "--dashboard");
    let dir = path.parent().unwrap().join("state");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("instance.lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|_| "MikroCockpit is already running with these settings")?;
    let mut builder = EventLoopBuilder::<AppEvent>::with_user_event();
    let mut event_loop = builder.build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
    let proxy = event_loop.create_proxy();
    native::init(event_loop.create_proxy());
    if let Ok(p) = fs::read_to_string(dir.join("preferences.json"))
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&p)
    {
        apply_preferences(&mut config, &v);
    }
    let shared = std::sync::Arc::new(std::sync::Mutex::new(
        serde_json::json!({"busy":false,"message":"Connecting…"}),
    ));
    // Authoritative "a web action is in flight" gate. The shared JSON is fully
    // rewritten on every rebuild, so it cannot hold this flag.
    let web_pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let dashboard = web::start(
        event_loop.create_proxy(),
        shared.clone(),
        web_pending.clone(),
        dir.clone(),
        config.clone(),
    )?;
    let (tx, rx) = mpsc::channel::<Request>();
    if args.iter().any(|a| a == "--print-url") {
        println!("{dashboard}");
    }
    let worker_config = config.clone();
    let worker_dir = dir.clone();
    thread::spawn(move || {
        let mut message = None;
        let mut failures = 0u32;
        let mut completed = false;
        let mut previous: Option<(Snapshot, Instant)> = None;
        loop {
            let mut snapshot = if demo_mode {
                Ok(demo())
            } else {
                worker_config.snapshot()
            };
            if let Ok(s) = &mut snapshot {
                let now = Instant::now();
                if let Some((old, then)) = &previous {
                    status::rates(s, old, now.duration_since(*then).as_secs_f64());
                    s.internet = old.internet;
                }
                // Publish router data before external reachability probes can time out.
                if !demo_mode {
                    let _ =
                        proxy.send_event(AppEvent::Updated(Box::new(Ok(s.clone())), None, false));
                }
                if !demo_mode {
                    match egress::probe(&worker_config, s) {
                        Ok((ip, source)) => {
                            s.public_ip = Some(ip);
                            s.public_ip_source = Some(source);
                        }
                        Err(e) => s.public_ip_error = Some(e),
                    }
                }
                s.internet = if demo_mode {
                    Some(true)
                } else {
                    Some(status::probe())
                };
                previous = Some((s.clone(), now));
            }
            failures = if snapshot.is_ok() {
                0
            } else {
                failures.saturating_add(1)
            };
            if let Ok(s) = &snapshot {
                // Atomic replace: readers must never observe a torn file.
                let final_path = worker_dir.join("snapshot.json");
                let tmp_path = worker_dir.join("snapshot.json.tmp");
                let written = serde_json::to_vec_pretty(s)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| fs::write(&tmp_path, bytes).map_err(|e| e.to_string()))
                    .and_then(|()| {
                        // Owner-only before the atomic rename; the final file inherits this mode.
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))
                                .map_err(|e| e.to_string())?;
                        }
                        fs::rename(&tmp_path, &final_path).map_err(|e| e.to_string())
                    });
                if let Err(e) = written {
                    // Not fatal for polling, but silence here hides a broken report button.
                    eprintln!("MikroCockpit: snapshot write failed: {e}");
                }
            }
            if proxy
                .send_event(AppEvent::Updated(
                    Box::new(snapshot),
                    message.take(),
                    completed,
                ))
                .is_err()
            {
                break;
            }
            completed = false;
            let delay = Duration::from_secs(
                (worker_config.poll_seconds * 2u64.pow(failures.min(3))).min(120),
            );
            match rx.recv_timeout(if memory_smoke {
                Duration::from_millis(10)
            } else {
                delay
            }) {
                Ok(Request::Refresh) => completed = true,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(request) => {
                    completed = true;
                    let description = match &request {
                        Request::Change(a) => format!("{a:?}"),
                        _ => "AddBypass".into(),
                    };
                    // Write intent first: a crash/timeout must not look like a confirmed action.
                    audit(
                        &worker_dir.join("audit.jsonl"),
                        &format!("requested: {description}"),
                    );
                    let result = if demo_mode {
                        Ok("Demo: router configuration was not changed".into())
                    } else {
                        perform(&worker_config, request, &worker_dir.join("audit.jsonl"))
                    };
                    message = Some(match result {
                        Ok(s) => s,
                        Err(e) => format!("Action error: {e}"),
                    });
                    audit(&worker_dir.join("audit.jsonl"), message.as_deref().unwrap());
                }
            }
        }
    });
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |e| {
        let _ = proxy.send_event(AppEvent::Menu(e));
    }));
    let mut tray: Option<TrayIcon> = None;
    let mut current = None;
    let mut error = None;
    let mut status = "Loading status…".to_string();
    let mut busy = false;
    let mut actions = HashMap::new();
    let began = Instant::now();
    let mut rendered_snapshot = false;
    let mut render_count = 0u64;
    let mut menu_refresh = menu_refresh::Refresh::default();
    let tracking = menu_refresh::Tracking::new();
    event_loop.run(move |event, _, flow| {
        // Dispatch queued selections against the old action IDs before replacing them.
        let menus_can_update = matches!(&event, Event::MainEventsCleared);
        *flow = if smoke {
            ControlFlow::WaitUntil(began + smoke_duration)
        } else if !rendered_snapshot {
            ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(250))
        } else {
            ControlFlow::Wait
        };
        let mut rebuild = false;
        match event {
            Event::NewEvents(StartCause::Init) => {
                let (m, a, t, tip) = build_menu(current.as_ref(), error.as_deref(), &status, busy, &config);
                actions = a;
                tray = Some(
                    TrayIconBuilder::new()
                        .with_menu(Box::new(m))
                        .with_icon_templated(icon())
                        .with_title(t)
                        .with_tooltip(tip)
                        .build()
                        .expect("macOS status item"),
                );
            }
            Event::UserEvent(AppEvent::Updated(result, message, completed)) => {
                match *result {
                    Ok(s) => {
                        current = Some(s);
                        error = None;
                        status = if demo_mode {
                            "Demo · no router changes".into()
                        } else {
                            "Status updated".into()
                        }
                    }
                    Err(e) => {
                        error = Some(e);
                        status = "No fresh data; controls are disabled".into()
                    }
                }
                if let Some(m) = message {
                    status = m
                }
                if completed {
                    busy = false
                }
                rebuild = true;
            }
            Event::UserEvent(AppEvent::NativePreferences(v))=>{
                match web::save_preferences(&dir,&v) { Ok(())=>{apply_preferences(&mut config,&v);status="Preferences saved".into();},Err(e)=>status=e }
                rebuild=true;
            },
            Event::UserEvent(AppEvent::Preferences(v))=>{apply_preferences(&mut config,&v);rebuild=true;},
            Event::UserEvent(AppEvent::Web(request))=>{
                // The gate has served its purpose: this event is being consumed now.
                web_pending.store(false, std::sync::atomic::Ordering::SeqCst);
                if !busy {
                    busy=true;status="Processing dashboard request…".into();
                    if tx.send(request).is_err() {
                        busy=false;status="Worker stopped".into();
                    }
                    rebuild=true;
                }
                else {shared.lock().unwrap()["message"]=serde_json::json!("Another action is already running");}
            }
            Event::UserEvent(AppEvent::Menu(e)) => match e.id.0.as_str() {
                "quit" => *flow = ControlFlow::Exit,
                "dashboard" => {native::update(&web::view(&shared,&dir,&config),true);},
                "web-dashboard" => {
                    // The auth token must not appear in `open` argv (readable via ps):
                    // hand the AppleScript with the full URL to osascript over stdin.
                    let script = format!(
                        "open location \"{}\"",
                        dashboard.replace('\\', "\\\\").replace('"', "\\\"")
                    );
                    if let Ok(mut child) = Command::new("/usr/bin/osascript")
                        .stdin(Stdio::piped())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                        && let Some(mut pipe) = child.stdin.take()
                    {
                        let _ = pipe.write_all(script.as_bytes());
                    }
                }
                "settings" => {
                    let _ = Command::new("/usr/bin/open")
                        .args(["-t"])
                        .arg(&path)
                        .spawn();
                }
                "audit" => {
                    let _ = Command::new("/usr/bin/open")
                        .args(["-t"])
                        .arg(dir.join("audit.jsonl"))
                        .spawn();
                }
                "report" => {
                    let _ = Command::new("/usr/bin/open")
                        .args(["-t"])
                        .arg(dir.join("snapshot.json"))
                        .spawn();
                }
                id => {
                    if !busy && let Some(request) = actions.get(id) {
                        busy = true;
                        status = "Working…".into();
                        if tx.send(request.clone()).is_err() {
                            // Worker is gone; never leave the menu locked on busy.
                            busy = false;
                            status = "Worker stopped".into()
                        }
                        rebuild = true;
                    }
                }
            },
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) if smoke => {
                assert!(rendered_snapshot, "Initial snapshot must reach the tray without manual Refresh");
                if native_smoke { native::capture(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/native-dashboard.png")); }
                eprintln!("MikroCockpit native tray smoke: OK ({render_count} renders)");
                *flow = ControlFlow::Exit;
            }
            _ => {}
        }
        if rebuild {
            *shared.lock().unwrap()=serde_json::json!({"snapshot":current,"error":error,"message":status,"busy":busy,"policy_ready":current.as_ref().is_some_and(|s|router::policy_ready(s,&config)),"updated":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()});
        }
        // Keep the open menu and its action IDs alive until native tracking ends.
        // Web state still updates immediately; only native presentation is deferred.
        if menu_refresh.take(rebuild, menus_can_update && !tracking.is_open()) {
            let (m, a, t, tip) =
                build_menu(current.as_ref(), error.as_deref(), &status, busy, &config);
            actions = a;
            render_count += 1;
            rendered_snapshot = current.is_some();
            if let Some(tray) = &tray {
                tray.set_menu(Some(Box::new(m)));
                tray.set_title(Some(t));
                let _ = tray.set_tooltip(Some(tip));
                let _ = tray.set_icon(Some(status::icon(current.as_ref(), error.is_some(), &config)));
                shared.lock().unwrap()["icon_state"] = serde_json::json!(status::icon_state(current.as_ref(),error.is_some(),&config));
                shared.lock().unwrap()["tray_rendered"] = serde_json::json!(true);
                shared.lock().unwrap()["status_line"] = serde_json::json!(status::title(current.as_ref(),error.as_deref(),&config));
                native::update(&web::view(&shared,&dir,&config),show_dashboard);
                show_dashboard = false;
            }
        }
        if menu_refresh.pending && !smoke && !matches!(*flow, ControlFlow::Exit) {
            *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100));
        }
    });
}
