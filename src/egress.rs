use crate::router::{Config, Result, Snapshot, flag, quote, rows, text};
use serde_json::Value;
use std::net::Ipv4Addr;

// The probe runs on the router. Never fall back to the Mac or the main table.
pub fn probe(config: &Config, snapshot: &Snapshot) -> Result<(String, String)> {
    let defaults: Vec<_> = snapshot
        .routes
        .iter()
        .filter(|r| {
            text(r, "dst-address") == "0.0.0.0/0" && flag(r, "active") && !flag(r, "disabled")
        })
        .collect();
    if defaults.len() != 1 || config.route_table == "main" {
        return Err("VPN probe requires one active default in a dedicated table".into());
    }
    let gateway = text(defaults[0], "gateway");
    let guard = format!(
        r#":local routes [/ip route find where routing-table={} and dst-address="0.0.0.0/0" and active=yes and disabled=no]; :if ([:len $routes] != 1) do={{:error "VPN route changed"}}; :if ([/ip route get [:pick $routes 0] gateway] != {}) do={{:error "VPN route changed"}}; "#,
        quote(&config.route_table),
        quote(&gateway)
    );
    let wireguard = snapshot.wg.iter().any(|w| text(w, "name") == gateway);
    let vrfs = read_rows(config, "/ip vrf print as-value proplist=name,disabled")?;
    let (command, source, container_shell) = if wireguard {
        let addresses = read_rows(config, "/ip address print as-value")?;
        let rules = read_rows(config, "/routing rule print as-value")?;
        let source_address =
            wireguard_probe_source(&config.route_table, &gateway, &addresses, &rules)?;
        (
            format!(
                r#":local result [/tool fetch url="https://api.ipify.org" src-address={} check-certificate=yes http-max-redirect-count=0 output=user as-value duration=8s]; :if (($result->"status") != "finished") do={{:error "VPN probe failed"}}; :put ($result->"data"); "#,
                quote(&source_address)
            ),
            format!("Router WireGuard {gateway}"),
            false,
        )
    } else if vrfs
        .iter()
        .any(|v| text(v, "name") == config.route_table && !flag(v, "disabled"))
    {
        (
            format!(
                r#":local result [/tool fetch url="https://api.ipify.org" address={} check-certificate=yes http-max-redirect-count=0 output=user as-value duration=8s]; :if (($result->"status") != "finished") do={{:error "VPN probe failed"}}; :put ($result->"data"); "#,
                quote(&format!("@{}", config.route_table))
            ),
            format!("Router VRF {}", config.route_table),
            false,
        )
    } else {
        let veths = read_rows(config, "/interface veth print as-value")?;
        let containers = read_rows(
            config,
            "/container print as-value proplist=name,interface,running",
        )?;
        let container = container_for_gateway(&gateway, &veths, &containers)?;
        // Fixed command only: no profile values are interpolated into the Linux shell.
        // timeout also bounds work if the SSH client disconnects.
        (
            format!(
                "/container shell {} cmd={}; ",
                quote(&text(container, ".id")),
                quote("timeout 8 wget -q -O - -T 5 https://api.ipify.org")
            ),
            format!("Router container {}", text(container, "name")),
            true,
        )
    };
    let output = if container_shell {
        // /container shell takes over the SSH channel: commands after it never run.
        let body = config.ssh_probe(&format!("{guard}{command}"))?;
        let end = config.ssh(&format!("{guard}:put \"MIKROCOCKPIT_ROUTE_OK\""))?;
        format!("{body}\nMIKROCOCKPIT_PROBE_END\n{end}")
    } else {
        config.ssh(&format!("{guard}{command}:put \"\\nMIKROCOCKPIT_PROBE_END\"; {guard}:put \"MIKROCOCKPIT_ROUTE_OK\""))?
    };
    Ok((parse_output(&output)?, source))
}
fn wireguard_probe_source(
    table: &str,
    gateway: &str,
    addresses: &[Value],
    rules: &[Value],
) -> Result<String> {
    let sources: Vec<_> = addresses
        .iter()
        .filter(|a| text(a, "interface") == gateway && !flag(a, "disabled") && !flag(a, "invalid"))
        .filter_map(|a| {
            let address = text(a, "address");
            address
                .split_once('/')
                .and_then(|(ip, _)| ip.parse::<Ipv4Addr>().ok())
                .map(|ip| (ip.to_string(), address))
        })
        .collect();
    if sources.len() != 1 {
        return Err("WireGuard probe needs one local IPv4 address".into());
    }
    let (source, network) = &sources[0];
    let protected = rules.iter().filter(|r| {
        !flag(r, "disabled")
            && text(r, "src-address") == *network
            && text(r, "action") == "lookup-only-in-table"
            && text(r, "table") == table
    });
    if protected.count() != 1 {
        return Err("WireGuard public-IP probe rule is missing".into());
    }
    Ok(source.clone())
}
fn read_rows(config: &Config, command: &str) -> Result<Vec<Value>> {
    let output = config.ssh(&format!(":put [:serialize to=json value=[{command}]]"))?;
    let value =
        serde_json::from_str(output.trim()).map_err(|_| "Router VPN probe facility unavailable")?;
    rows(&value)
}
fn container_for_gateway<'a>(
    gateway: &str,
    veths: &[Value],
    containers: &'a [Value],
) -> Result<&'a Value> {
    let gateway: Ipv4Addr = gateway
        .parse()
        .map_err(|_| "VPN probe needs a VRF or a container gateway")?;
    let names: Vec<_> = veths
        .iter()
        .filter(|v| {
            !flag(v, "disabled")
                && flag(v, "running")
                && v["address"].as_array().is_some_and(|a| {
                    a.iter().any(|a| {
                        a.as_str()
                            .and_then(|s| s.split('/').next())
                            .and_then(|s| s.parse::<Ipv4Addr>().ok())
                            == Some(gateway)
                    })
                })
        })
        .map(|v| text(v, "name"))
        .collect();
    let matches: Vec<_> = containers
        .iter()
        .filter(|c| {
            flag(c, "running")
                && c["interface"].as_array().is_some_and(|a| {
                    a.iter().any(|n| {
                        n.as_str()
                            .is_some_and(|n| names.iter().any(|name| name == n))
                    })
                })
        })
        .collect();
    if matches.len() != 1 {
        return Err("No unique running VPN container matches the active gateway".into());
    }
    Ok(matches[0])
}
fn parse_output(output: &str) -> Result<String> {
    let (body, end) = output
        .split_once("MIKROCOCKPIT_PROBE_END")
        .ok_or("VPN probe did not complete")?;
    if end.trim() != "MIKROCOCKPIT_ROUTE_OK" {
        return Err("VPN route changed during probe".into());
    }
    body.trim()
        .parse::<Ipv4Addr>()
        .map(|ip| ip.to_string())
        .map_err(|_| "VPN probe did not return an IPv4 address".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn router_probe_rejects_wrong_paths_and_failed_results() {
        let veths = vec![json!({"name":"veth-vpn","running":true,"address":["172.20.0.2/24"]})];
        let containers =
            vec![json!({".id":"*1","name":"vpn","running":true,"interface":["veth-vpn"]})];
        assert_eq!(
            text(
                container_for_gateway("172.20.0.2", &veths, &containers).unwrap(),
                "name"
            ),
            "vpn"
        );
        assert!(container_for_gateway("192.168.1.1", &veths, &containers).is_err());
        assert!(container_for_gateway("wg0", &veths, &containers).is_err());
        assert!(
            container_for_gateway(
                "172.20.0.2",
                &veths,
                &[containers[0].clone(), containers[0].clone()]
            )
            .is_err()
        );
        let addresses = vec![json!({"address":"10.15.13.167/32","interface":"wg-fin"})];
        let rules = vec![
            json!({"src-address":"10.15.13.167/32","action":"lookup-only-in-table","table":"proxy"}),
        ];
        assert_eq!(
            wireguard_probe_source("proxy", "wg-fin", &addresses, &rules).unwrap(),
            "10.15.13.167"
        );
        assert!(wireguard_probe_source("proxy", "wg-fin", &addresses, &[]).is_err());
        assert_eq!(
            parse_output("203.0.113.42\r\nMIKROCOCKPIT_PROBE_END\r\nMIKROCOCKPIT_ROUTE_OK\r\n")
                .unwrap(),
            "203.0.113.42"
        );
        assert!(parse_output("203.0.113.42\nMIKROCOCKPIT_PROBE_END\nVPN route changed").is_err());
        assert!(
            parse_output("wget: failed\nMIKROCOCKPIT_PROBE_END\nMIKROCOCKPIT_ROUTE_OK").is_err()
        );
        assert!(parse_output("203.0.113.42").is_err());
    }
}
