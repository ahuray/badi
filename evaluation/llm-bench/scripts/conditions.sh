#!/usr/bin/env bash
# conditions.sh LABEL — append one JSON line of test conditions to results/conditions.jsonl.
label=${1:-snapshot}
data=$(python3 -I "$(dirname "$0")/bench_paths.py" data)
mkdir -p "$data/results"
ac=$(cat /sys/class/power_supply/AC*/online /sys/class/power_supply/ADP*/online 2>/dev/null | head -1)
prof=$(powerprofilesctl get 2>/dev/null)
gov=$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor)
epp=$(cat /sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference 2>/dev/null)
load=$(cut -d' ' -f1-3 /proc/loadavg)
avail=$(awk '/MemAvailable/{print $2*1024}' /proc/meminfo)
temp=$(for z in /sys/class/thermal/thermal_zone*; do [[ $(cat $z/type) == x86_pkg_temp ]] && cat $z/temp; done | head -1)
badi_req=$(badi status 2>/dev/null | sed -n 's/^Requests: \([0-9]*\).*/\1/p')
busy=$(ps -eo pcpu=,comm= --sort=-pcpu | head -3 | awk '{printf "%s:%s ", $2, $1}')
printf '{"t":"%s","label":"%s","ac_online":"%s","power_profile":"%s","governor":"%s","epp":"%s","loadavg":"%s","mem_available":%s,"pkg_temp_mC":"%s","badi_requests":"%s","top_cpu":"%s"}\n' \
  "$(date -Iseconds)" "$label" "$ac" "$prof" "$gov" "$epp" "$load" "$avail" "$temp" "$badi_req" "$busy" | tee -a "$data/results/conditions.jsonl"
