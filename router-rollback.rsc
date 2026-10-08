# Remove only MikroCockpit rules; restore the pre-existing policy, not a factory reset.
/ip firewall mangle remove [find where comment="MikroCockpit:v1:direct"]
/ip firewall filter remove [find where comment="MikroCockpit:v1:guard"]
/ip firewall filter remove [find where chain="mikrocockpit-guard" and comment~"^MikroCockpit:v1:"]
/ipv6 firewall filter remove [find where comment="MikroCockpit:v1:ipv6"]
/ip firewall address-list remove [find where list="mikrocockpit-direct" and comment="MikroCockpit"]
/interface list member remove [find where list="MikroCockpit-WAN" and comment="MikroCockpit:v1"]
/interface list remove [find where name="MikroCockpit-WAN" and comment="MikroCockpit:v1"]
