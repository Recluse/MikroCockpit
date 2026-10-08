# Generated MikroCockpit policy v1. Review, approve, then import. No VPN switch.
# IPv4 direct exceptions only; forwarded WAN IPv6 remains blocked.
:if ([:len [/ip firewall mangle find where chain=prerouting and action=mark-routing and new-routing-mark="proxy" and in-interface-list="LAN" and dst-address-list="!vpn-bypass" and disabled=no]] != 1) do={:error "VPN marking does not match profile"}
:if ([:len [/interface find where name="ether1"]] != 1) do={:error "WAN interface missing"}
:if ([:len [/ip firewall filter find where action=fasttrack-connection and disabled=no]] > 0) do={:error "Fasttrack must be disabled"}
:if ([:len [/ip firewall filter find where comment~"^MikroCockpit:"]] > 0) do={:error "Policy already or partially installed"}
:if ([:len [/ip firewall mangle find where comment~"^MikroCockpit:"]] > 0) do={:error "Policy already or partially installed"}
:if ([:len [/ipv6 firewall filter find where comment~"^MikroCockpit:"]] > 0) do={:error "Policy already or partially installed"}
:if ([:len [/ip firewall filter find where chain="mikrocockpit-guard"]] > 0) do={:error "Guard chain already exists"}
:if ([:len [/interface list find where name="MikroCockpit-WAN"]] > 0) do={:error "MikroCockpit-WAN already exists"}
:if ([:len [/ip firewall address-list find where list="mikrocockpit-direct"]] > 0) do={:error "Direct list must start empty"}
/system backup save name=pre-mikrocockpit
/export file=pre-mikrocockpit
:do {
/interface list add name="MikroCockpit-WAN" comment="MikroCockpit:v1"
/interface list member add list="MikroCockpit-WAN" interface="ether1" comment="MikroCockpit:v1"
/ip firewall filter add action="accept" chain="mikrocockpit-guard" comment="MikroCockpit:v1:device" src-address-list="mikrocockpit-direct"
/ip firewall filter add action="accept" chain="mikrocockpit-guard" comment="MikroCockpit:v1:destination" dst-address-list="vpn-bypass"
/ip firewall filter add action="drop" chain="mikrocockpit-guard" comment="MikroCockpit:v1:drop"
/ip firewall filter add action="jump" chain="forward" comment="MikroCockpit:v1:guard" in-interface-list="LAN" jump-target="mikrocockpit-guard" out-interface-list="MikroCockpit-WAN" place-before=0
/ip firewall mangle add action="accept" chain="prerouting" comment="MikroCockpit:v1:direct" in-interface-list="LAN" src-address-list="mikrocockpit-direct" place-before=0
/ipv6 firewall filter add action="drop" chain="forward" comment="MikroCockpit:v1:ipv6" in-interface-list="LAN" out-interface-list="MikroCockpit-WAN" place-before=0
} on-error={
# Remove only MikroCockpit rules; restore the pre-existing policy, not a factory reset.
/ip firewall mangle remove [find where comment="MikroCockpit:v1:direct"]
/ip firewall filter remove [find where comment="MikroCockpit:v1:guard"]
/ip firewall filter remove [find where chain="mikrocockpit-guard" and comment~"^MikroCockpit:v1:"]
/ipv6 firewall filter remove [find where comment="MikroCockpit:v1:ipv6"]
/ip firewall address-list remove [find where list="mikrocockpit-direct" and comment="MikroCockpit"]
/interface list member remove [find where list="MikroCockpit-WAN" and comment="MikroCockpit:v1"]
/interface list remove [find where name="MikroCockpit-WAN" and comment="MikroCockpit:v1"]
:error "Policy installation failed; removed MikroCockpit additions"
}
:put "MikroCockpit policy installed; direct list remains empty"
