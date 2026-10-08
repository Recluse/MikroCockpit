use crate::{
    AppEvent, Request,
    router::{Action, Config, Result},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::PathBuf,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use tao::event_loop::EventLoopProxy;
use tiny_http::{Header, Method, Response, Server};

pub type State = Arc<Mutex<Value>>;
#[derive(Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum WebAction {
    #[serde(rename = "refresh")]
    Refresh,
    #[serde(rename = "exit")]
    Exit { name: String },
    #[serde(rename = "direct")]
    Direct {
        address: String,
        minutes: Option<u32>,
    },
    #[serde(rename = "end_direct")]
    EndDirect { address: String },
    #[serde(rename = "wireguard")]
    Wireguard { name: String, enabled: bool },
    #[serde(rename = "primary")]
    Primary { id: String },
    #[serde(rename = "add_bypass")]
    AddBypass { address: String },
    #[serde(rename = "remove_bypass")]
    RemoveBypass { id: String, address: String },
}
pub fn decode(body: &str) -> Result<Request> {
    let value: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    if value["kind"] == "direct" && value.get("minutes").is_none() {
        return Err("Specify a duration explicitly; null means permanent".into());
    }
    let a: WebAction = serde_json::from_value(value).map_err(|e| e.to_string())?;
    Ok(match a {
        WebAction::Refresh => Request::Refresh,
        WebAction::Exit { name } => Request::Change(Action::Exit(name)),
        WebAction::Direct { address, minutes } => Request::Change(Action::Direct(address, minutes)),
        WebAction::EndDirect { address } => Request::Change(Action::EndDirect(address)),
        WebAction::Wireguard { name, enabled } => Request::Change(Action::Wireguard(name, enabled)),
        WebAction::Primary { id } => Request::Change(Action::Primary(id)),
        WebAction::AddBypass { address } => Request::Change(Action::AddBypass(address)),
        WebAction::RemoveBypass { id, address } => {
            Request::Change(Action::RemoveBypass(id, address))
        }
    })
}
fn header<'a>(r: &'a tiny_http::Request, name: &'static str) -> Option<&'a str> {
    r.headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str())
}
fn auth(
    host: Option<&str>,
    origin: Option<&str>,
    token: Option<&str>,
    expected_host: &str,
    expected_token: &str,
    write: bool,
) -> bool {
    host == Some(expected_host)
        && token == Some(expected_token)
        && (!write || origin == Some(format!("http://{expected_host}").as_str()))
}
pub fn preferences(v: &Value) -> bool {
    let fields_ok = v.get("status_fields").is_none_or(|a| {
        a.as_array().is_some_and(|a| {
            a.len() <= crate::status::FIELDS.len()
                && a.iter().all(|v| {
                    v.as_str()
                        .is_some_and(|s| crate::status::FIELDS.contains(&s))
                })
        })
    });
    let interfaces_ok = v.get("status_interfaces").is_none_or(|a| {
        a.as_array().is_some_and(|a| {
            a.len() <= 8
                && a.iter().all(|v| {
                    v.as_str().is_some_and(|s| {
                        !s.is_empty() && s.len() <= 64 && !s.chars().any(char::is_control)
                    })
                })
        })
    });
    if !fields_ok || !interfaces_ok {
        return false;
    }

    (v.get("icon_color").is_none()
        || matches!(
            v["icon_color"].as_str(),
            Some("system" | "monochrome" | "blue" | "purple" | "green" | "orange" | "pink")
        ))
        && matches!(v["theme"].as_str(), Some("dark" | "light" | "system"))
        && v["hidden"].as_array().is_some_and(|a| {
            a.len() <= 4
                && a.iter().all(|v| {
                    matches!(
                        v.as_str(),
                        Some("devices" | "routes" | "bypass" | "wireguard")
                    )
                })
        })
        && v["tray_order"].as_array().is_some_and(|a| {
            a.len() <= 5
                && a.iter().all(|v| {
                    matches!(
                        v.as_str(),
                        Some("direct" | "exits" | "routes" | "wireguard" | "bypass")
                    )
                })
        })
        && matches!(
            v["tray_title"].as_str(),
            Some("exit" | "cpu" | "name" | "icon" | "custom")
        )
}
pub fn view(state: &State, dir: &std::path::Path, config: &Config) -> Value {
    let mut s = state.lock().unwrap().clone();
    s["locale"] = json!(crate::locale::current());
    s["config"] = json!({"host":config.host,"group":config.mihomo_group,"table":config.route_table,"lan":config.lan_cidrs,"protected":config.protected_wireguard,"name":config.name,"wireguard":config.managed_wireguard});
    s["preferences"]=fs::read_to_string(dir.join("preferences.json")).ok().and_then(|t|serde_json::from_str::<Value>(&t).ok()).filter(preferences).unwrap_or(json!({"theme":config.theme,"icon_color":config.icon_color,"hidden":[],"tray_order":config.tray_order,"tray_title":config.tray_title,"status_fields":config.status_fields,"status_interfaces":config.status_interfaces}));
    s
}
pub fn save_preferences(dir: &std::path::Path, value: &Value) -> Result<()> {
    if !preferences(value) {
        return Err("Invalid dashboard preferences".into());
    }
    fs::write(dir.join("preferences.tmp"), value.to_string()).map_err(|e| e.to_string())?;
    fs::rename(dir.join("preferences.tmp"), dir.join("preferences.json")).map_err(|e| e.to_string())
}
pub fn start(
    proxy: EventLoopProxy<AppEvent>,
    state: State,
    web_pending: Arc<AtomicBool>,
    dir: PathBuf,
    config: Config,
) -> Result<String> {
    let server = Server::http("127.0.0.1:0").map_err(|e| e.to_string())?;
    let host = server.server_addr().to_string();
    let token = crate::router::run(
        &mut Command::new("/usr/bin/uuidgen"),
        Duration::from_secs(2),
    )?
    .trim()
    .to_string();
    let url = format!("http://{host}/#{token}");
    // Fragment never reaches the HTTP server or appears in referrer headers.
    thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let path = request.url().to_string();
            let method = request.method().clone();
            let is_api = path.starts_with("/api/");
            let authorized = auth(
                header(&request, "Host"),
                header(&request, "Origin"),
                header(&request, "X-MikroCockpit-Token"),
                &host,
                &token,
                method == Method::Post,
            );
            let (code, content_type, body) =
                if header(&request, "Host") != Some(host.as_str()) || (is_api && !authorized) {
                    (403, "text/plain; charset=utf-8", "Forbidden".into())
                } else if method == Method::Get {
                    match path.as_str() {
                        "/" => (
                            200,
                            "text/html; charset=utf-8",
                            include_str!("../ui/index.html").into(),
                        ),
                        "/app.js" => (
                            200,
                            "text/javascript; charset=utf-8",
                            include_str!("../ui/app.js").into(),
                        ),
                        "/style.css" => (
                            200,
                            "text/css; charset=utf-8",
                            include_str!("../ui/style.css").into(),
                        ),
                        "/logo.png" => (200, "image/png", String::new()),
                        "/api/state" => {
                            let s = view(&state, &dir, &config);
                            (200, "application/json", s.to_string())
                        }
                        _ => (404, "text/plain", "Not found".into()),
                    }
                } else if method == Method::Post && is_api {
                    if request.body_length().is_none_or(|n| n > 4096)
                        || header(&request, "Content-Type") != Some("application/json")
                    {
                        (400, "text/plain", "Expected small JSON body".into())
                    } else {
                        let mut body = String::new();
                        let result = request
                        .as_reader()
                        .take(4097)
                        .read_to_string(&mut body)
                        .map_err(|e| e.to_string())
                        .and_then(|_| {
                            if path == "/api/action" {
                                let request = decode(&body)?;
                                // web_pending is the authoritative gate: the event-loop
                                // rebuild below overwrites the whole shared JSON, so a
                                // busy flag stored there can be clobbered before the
                                // Web event is processed.
                                if web_pending.load(Ordering::SeqCst) {
                                    return Err("An action is already running".into());
                                }
                                let mut state = state.lock().unwrap();
                                if state["busy"] == true {
                                    return Err("An action is already running".into());
                                }
                                web_pending.store(true, Ordering::SeqCst);
                                if let Err(e) = proxy.send_event(AppEvent::Web(request)) {
                                    web_pending.store(false, Ordering::SeqCst);
                                    return Err(e.to_string());
                                }
                                state["busy"] = json!(true);
                                state["message"] = json!(
                                    "Request sent; confirm the action in the MikroCockpit window"
                                );
                                Ok(())
                            } else if path == "/api/preferences" {
                                let value: Value =
                                    serde_json::from_str(&body).map_err(|e| e.to_string())?;
                                save_preferences(&dir, &value)?;
                                proxy
                                    .send_event(AppEvent::Preferences(value))
                                    .map_err(|_| "Application closed".to_string())
                            } else {
                                Err("Unknown endpoint".into())
                            }
                        });
                        match result {
                            Ok(()) => (202, "application/json", json!({"ok":true}).to_string()),
                            Err(e) => (400, "application/json", json!({"error":e}).to_string()),
                        }
                    }
                } else {
                    (405, "text/plain", "Method not allowed".into())
                };
            let bytes = if content_type == "image/png" {
                include_bytes!("../assets/logo.png").to_vec()
            } else {
                body.into_bytes()
            };
            let mut response = Response::from_data(bytes).with_status_code(code);
            for (name, value) in [
                ("Content-Type", content_type),
                ("Cache-Control", "no-store"),
                ("X-Content-Type-Options", "nosniff"),
                ("Referrer-Policy", "no-referrer"),
                (
                    "Content-Security-Policy",
                    "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
                ),
            ] {
                response.add_header(Header::from_bytes(name, value).unwrap());
            }
            let _ = request.respond(response);
        }
    });
    Ok(url)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn web_rejects_cross_origin_and_arbitrary_commands() {
        assert!(auth(
            Some("127.0.0.1:123"),
            Some("http://127.0.0.1:123"),
            Some("secret"),
            "127.0.0.1:123",
            "secret",
            true
        ));
        assert!(!auth(
            Some("attacker.test"),
            None,
            Some("secret"),
            "127.0.0.1:123",
            "secret",
            false
        ));
        assert!(!auth(
            Some("127.0.0.1:123"),
            Some("https://attacker.test"),
            Some("secret"),
            "127.0.0.1:123",
            "secret",
            true
        ));
        assert!(!auth(
            Some("127.0.0.1:123"),
            None,
            None,
            "127.0.0.1:123",
            "secret",
            false
        ));
        assert!(decode(r#"{"kind":"command","script":"/system reboot"}"#).is_err());
        assert!(decode(r#"{"kind":"exit","name":"AMS-grpc","script":"evil"}"#).is_err());
        assert!(decode(r#"{"kind":"direct","address":"192.0.2.5","minutes":15}"#).is_ok());
    }
}
