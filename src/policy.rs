use crate::router::{Config, Result, Snapshot, flag, quote, text};
use serde_json::{Value, json};

const WAN: &str = "MikroCockpit-WAN";
pub fn guard(c: &Config) -> Value {
    json!({"chain":"forward","action":"jump","jump-target":"mikrocockpit-guard","in-interface-list":c.lan_interface_list,"out-interface-list":WAN,"comment":"MikroCockpit:v1:guard"})
}
pub fn mangle(c: &Config) -> Value {
    json!({"chain":"prerouting","action":"accept","src-address-list":"mikrocockpit-direct","in-interface-list":c.lan_interface_list,"comment":"MikroCockpit:v1:direct"})
}
pub fn ipv6(c: &Config) -> Value {
    json!({"chain":"forward","action":"drop","in-interface-list":c.lan_interface_list,"out-interface-list":WAN,"comment":"MikroCockpit:v1:ipv6"})
}
pub fn chain(c: &Config) -> Vec<Value> {
    let mut rules = vec![
        json!({"chain":"mikrocockpit-guard","action":"accept","src-address-list":"mikrocockpit-direct","comment":"MikroCockpit:v1:device"}),
        json!({"chain":"mikrocockpit-guard","action":"accept","dst-address-list":c.bypass_list,"comment":"MikroCockpit:v1:destination"}),
    ];
    for (i, extra) in c.extra_direct_rules.iter().enumerate() {
        let mut r = json!({"chain":"mikrocockpit-guard","action":"accept","comment":format!("MikroCockpit:v1:exception-{i}")});
        for (k, v) in extra {
            r[k] = json!(v)
        }
        rules.push(r);
    }
    rules.push(
        json!({"chain":"mikrocockpit-guard","action":"drop","comment":"MikroCockpit:v1:drop"}),
    );
    rules
}
fn matches(actual: &Value, expected: &Value) -> bool {
    if flag(actual, "disabled") || flag(actual, "invalid") {
        return false;
    }
    let Some(a) = actual.as_object() else {
        return false;
    };
    let e = expected.as_object().unwrap();
    e.iter().all(|(k, v)| actual[k] == *v)
        && a.keys().all(|k| {
            e.contains_key(k)
                || [
                    ".id",
                    "disabled",
                    "invalid",
                    "dynamic",
                    "log",
                    "log-prefix",
                    "passthrough",
                ]
                .contains(&k.as_str())
        })
}
pub fn ready(s: &Snapshot, c: &Config) -> bool {
    let first = |rows: &Vec<Value>, chain: &str| {
        rows.iter()
            .find(|r| !flag(r, "disabled") && text(r, "chain") == chain)
            .cloned()
    };
    let marks: Vec<_> = s
        .mangle
        .iter()
        .filter(|r| {
            !flag(r, "disabled")
                && !flag(r, "invalid")
                && text(r, "chain") == "prerouting"
                && text(r, "action") == "mark-routing"
                && text(r, "new-routing-mark") == c.route_table
                && text(r, "in-interface-list") == c.lan_interface_list
                && text(r, "dst-address-list") == format!("!{}", c.bypass_list)
                && r.get("passthrough").is_some()
                && !flag(r, "passthrough")
        })
        .collect();
    if marks.len() != 1
        || !first(&s.filter, "forward").is_some_and(|r| matches(&r, &guard(c)))
        || !first(&s.mangle, "prerouting").is_some_and(|r| matches(&r, &mangle(c)))
        || !first(&s.ipv6_filter, "forward").is_some_and(|r| matches(&r, &ipv6(c)))
    {
        return false;
    }
    if s.filter
        .iter()
        .any(|r| !flag(r, "disabled") && text(r, "action") == "fasttrack-connection")
    {
        return false;
    }
    let actual: Vec<_> = s
        .filter
        .iter()
        .filter(|r| !flag(r, "disabled") && text(r, "chain") == "mikrocockpit-guard")
        .collect();
    let expected = chain(c);
    let mut members: Vec<_> = s
        .members
        .iter()
        .filter(|r| text(r, "list") == WAN && !flag(r, "disabled"))
        .map(|r| text(r, "interface"))
        .collect();
    members.sort();
    let mut wanted = c.wan_interfaces.clone();
    wanted.sort();
    !wanted.is_empty()
        && members == wanted
        && actual.len() == expected.len()
        && actual.iter().zip(&expected).all(|(a, e)| matches(a, e))
}
fn add(path: &str, r: &Value, first: bool) -> String {
    let mut s = format!("{path} add");
    for (k, v) in r.as_object().unwrap() {
        s.push_str(&format!(" {k}={}", quote(v.as_str().unwrap())))
    }
    if first {
        s.push_str(" place-before=0")
    }
    s.push('\n');
    s
}
pub fn setup(c: &Config) -> Result<String> {
    c.validate()?;
    if c.route_table == "main" || c.wan_interfaces.is_empty() {
        return Err("Choose a dedicated VPN table and WAN interfaces in the profile".into());
    }
    let mut s = String::from(
        "# Generated MikroCockpit policy v1. Review, approve, then import. No VPN switch.\n# IPv4 direct exceptions only; forwarded WAN IPv6 remains blocked.\n",
    );
    s.push_str(&format!(":if ([:len [/ip firewall mangle find where chain=prerouting and action=mark-routing and new-routing-mark={} and in-interface-list={} and dst-address-list={} and passthrough=no and disabled=no]] != 1) do={{:error \"VPN marking does not match profile\"}}\n",quote(&c.route_table),quote(&c.lan_interface_list),quote(&format!("!{}",c.bypass_list))));
    for interface in &c.wan_interfaces {
        s.push_str(&format!(":if ([:len [/interface find where name={}]] != 1) do={{:error \"WAN interface missing\"}}\n",quote(interface)));
    }
    s.push_str(":if ([:len [/ip firewall filter find where action=fasttrack-connection and disabled=no]] > 0) do={:error \"Fasttrack must be disabled\"}\n");
    for path in [
        "/ip firewall filter",
        "/ip firewall mangle",
        "/ipv6 firewall filter",
    ] {
        s.push_str(&format!(":if ([:len [{path} find where comment~\"^MikroCockpit:\"]] > 0) do={{:error \"Policy already or partially installed\"}}\n"));
    }
    s.push_str(":if ([:len [/ip firewall filter find where chain=\"mikrocockpit-guard\"]] > 0) do={:error \"Guard chain already exists\"}\n:if ([:len [/interface list find where name=\"MikroCockpit-WAN\"]] > 0) do={:error \"MikroCockpit-WAN already exists\"}\n:if ([:len [/ip firewall address-list find where list=\"mikrocockpit-direct\"]] > 0) do={:error \"Direct list must start empty\"}\n/system backup save name=pre-mikrocockpit\n/export file=pre-mikrocockpit\n:do {\n/interface list add name=\"MikroCockpit-WAN\" comment=\"MikroCockpit:v1\"\n");
    for interface in &c.wan_interfaces {
        s.push_str(&format!("/interface list member add list=\"MikroCockpit-WAN\" interface={} comment=\"MikroCockpit:v1\"\n",quote(interface)));
    }
    for rule in chain(c) {
        s.push_str(&add("/ip firewall filter", &rule, false));
    }
    s.push_str(&add("/ip firewall filter", &guard(c), true));
    s.push_str(&add("/ip firewall mangle", &mangle(c), true));
    s.push_str(&add("/ipv6 firewall filter", &ipv6(c), true));
    s.push_str("} on-error={\n");
    s.push_str(include_str!("../router-rollback.rsc"));
    s.push_str(":error \"Policy installation failed; removed MikroCockpit additions\"\n}\n:put \"MikroCockpit policy installed; direct list remains empty\"\n");
    Ok(s)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generic_policy_rejects_leaks_and_wrong_interfaces() {
        let c: Config = serde_json::from_str(include_str!("../settings.example.json")).unwrap();
        let mut s = crate::demo();
        s.filter = vec![guard(&c)];
        s.filter.extend(chain(&c));
        s.mangle = vec![mangle(&c)];
        s.mangle.push(json!({"chain":"prerouting","action":"mark-routing","new-routing-mark":"proxy","in-interface-list":"LAN","dst-address-list":"!vpn-bypass","passthrough":false}));
        s.ipv6_filter = vec![ipv6(&c)];
        s.members = vec![json!({"list":WAN,"interface":"ether1"})];
        assert!(ready(&s, &c));
        let mut bad = s.clone();
        bad.mangle.pop();
        assert!(!ready(&bad, &c));
        let mut bad = s.clone();
        bad.filter.insert(
            0,
            json!({"chain":"forward","action":"accept","connection-state":"established,related"}),
        );
        assert!(!ready(&bad, &c));
        let mut bad = s.clone();
        bad.filter.last_mut().unwrap()["protocol"] = json!("tcp");
        assert!(!ready(&bad, &c));
        let mut bad = s.clone();
        bad.members[0]["interface"] = json!("wrong-wan");
        assert!(!ready(&bad, &c));
        s.ipv6_filter[0]["disabled"] = json!(true);
        assert!(!ready(&s, &c));
        let plan = setup(&c).unwrap();
        assert!(plan.contains("MikroCockpit-WAN"));
        assert!(plan.contains("} on-error={\n"));
        assert!(plan.contains(include_str!("../router-rollback.rsc")));
        assert!(plan.contains("chain=prerouting and action=mark-routing"));
        assert!(!plan.contains("address-list add"));
        // The generated plan must never embed LAN addresses.
        assert!(!plan.contains("192.168.88"));
    }
}
