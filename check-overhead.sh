#!/usr/bin/env bash
# check-overhead.sh — measures how much CPU and memory ccmux uses
#
# Usage:  ./check-overhead.sh [duration_seconds]
# Default: 20 seconds of sampling

DURATION=${1:-20}
INTERVAL=2

# ── colours ──────────────────────────────────────────────────────────────
GREEN='\033[0;32m'; YELLOW='\033[0;33m'; RED='\033[0;31m'
CYAN='\033[0;36m'; DIM='\033[2m'; BOLD='\033[1m'; RESET='\033[0m'

# ── helpers ───────────────────────────────────────────────────────────────
ok()   { echo -e "  ${GREEN}✓${RESET}  $*"; }
warn() { echo -e "  ${YELLOW}⚠${RESET}  $*"; }
err()  { echo -e "  ${RED}✗${RESET}  $*"; }
hdr()  { echo -e "\n${BOLD}── $* ${DIM}$(printf '─%.0s' {1..50})${RESET}"; echo ""; }

# time a command in milliseconds using python3 (ms precision, macOS-safe)
time_ms() {
    python3 -c "
import time, subprocess, sys
start = time.time()
subprocess.run(sys.argv[1:], capture_output=True)
print(int((time.time() - start) * 1000))
" -- "$@" 2>/dev/null
}

# average time_ms over N runs
avg_ms() {
    local n=$1; shift
    local total=0 t
    for _ in $(seq 1 "$n"); do
        t=$(time_ms "$@")
        total=$((total + t))
    done
    echo $((total / n))
}

# ── find process ──────────────────────────────────────────────────────────
hdr "Finding ccmux process"

PID=$(pgrep -x ccmux 2>/dev/null | head -1)
if [[ -z "$PID" ]]; then
    err "ccmux is not running."
    echo "     Start it first: open a tmux pane and run 'ccmux sidebar'"
    exit 1
fi

CMD=$(ps -p "$PID" -o args= | head -1)
echo -e "  ${CYAN}PID${RESET} $PID  →  $CMD"

# ── CPU / memory sampling ─────────────────────────────────────────────────
hdr "CPU & Memory — sampling for ${DURATION}s"
printf "  ${DIM}%-6s  %-9s  %-10s${RESET}\n" "sample" "CPU%" "memory"
printf "  ${DIM}%-6s  %-9s  %-10s${RESET}\n" "------" "----" "------"

total_cpu=0; total_rss=0; max_rss=0
count=0; samples=$((DURATION / INTERVAL))

for i in $(seq 1 "$samples"); do
    row=$(ps -p "$PID" -o pcpu=,rss= 2>/dev/null | tr -s ' ' | sed 's/^ //')
    [[ -z "$row" ]] && { echo "  (process ended)"; break; }
    cpu=$(echo "$row" | awk '{print $1}')
    rss=$(echo "$row" | awk '{print $2}')

    rss_mb=$(awk "BEGIN{printf \"%.1f\", $rss/1024}")

    # colour CPU: green <1%, yellow 1-5%, red >5%
    if awk "BEGIN{exit !($cpu >= 5)}"; then
        cpu_str="${RED}${cpu}%${RESET}"
    elif awk "BEGIN{exit !($cpu >= 1)}"; then
        cpu_str="${YELLOW}${cpu}%${RESET}"
    else
        cpu_str="${GREEN}${cpu}%${RESET}"
    fi

    printf "  %-6d  %-18b  %s MB\n" "$i" "$cpu_str" "$rss_mb"

    total_cpu=$(awk "BEGIN{print $total_cpu + $cpu}")
    total_rss=$(awk "BEGIN{print $total_rss + $rss}")
    [[ $rss -gt $max_rss ]] && max_rss=$rss
    count=$((count + 1))
    sleep "$INTERVAL"
done

avg_cpu=$(awk "BEGIN{printf \"%.2f\", $total_cpu / $count}")
avg_rss_mb=$(awk "BEGIN{printf \"%.1f\", $total_rss / 1024 / $count}")
max_rss_mb=$(awk "BEGIN{printf \"%.1f\", $max_rss / 1024}")

echo ""
echo -e "  Avg CPU:    ${BOLD}${avg_cpu}%${RESET}"
echo -e "  Max CPU:    ${BOLD}$(awk "BEGIN{printf \"%.2f\", $max_rss}")%${RESET}" 2>/dev/null || true
echo -e "  Avg memory: ${BOLD}${avg_rss_mb} MB${RESET}"
echo -e "  Max memory: ${BOLD}${max_rss_mb} MB${RESET}"

# ── sub-operation timings ─────────────────────────────────────────────────
hdr "Sub-operation timings (what ccmux runs each refresh)"

SESSION=$(tmux display-message -p '#{session_name}' 2>/dev/null)
FIRST_PANE=$(tmux list-panes -s -F '#{pane_id}' 2>/dev/null | head -1)

if [[ -z "$SESSION" ]]; then
    warn "Not inside tmux — skipping sub-operation timings."
else
    echo -e "  ${DIM}(each timed as average of 5 runs)${RESET}\n"

    ps_ms=$(avg_ms 5 ps -eo pid,ppid,comm)
    list_ms=$(avg_ms 5 tmux list-panes -s -t "$SESSION" -F '#{pane_id}')
    cap_ms=$(avg_ms 5 tmux capture-pane -p -t "$FIRST_PANE" -l 30)

    printf "  %-35s %dms\n" "ps -eo pid,ppid,comm" "$ps_ms"
    printf "  %-35s %dms\n" "tmux list-panes -s" "$list_ms"
    printf "  %-35s %dms\n" "tmux capture-pane (1 session)" "$cap_ms"

    # Estimate total refresh cost
    N_SESSIONS=$(tmux list-panes -s -F '#{pane_id}' 2>/dev/null | wc -l | tr -d ' ')
    total_refresh=$((ps_ms + list_ms + cap_ms * N_SESSIONS))
    echo ""
    echo -e "  ${DIM}With $N_SESSIONS session(s): estimated ${total_refresh}ms per full refresh cycle${RESET}"
fi

# ── verdict ───────────────────────────────────────────────────────────────
hdr "Verdict"

heavy=0

if awk "BEGIN{exit !($avg_cpu >= 5)}"; then
    warn "CPU is HIGH: avg ${avg_cpu}%  (expected < 1%)"
    heavy=1
elif awk "BEGIN{exit !($avg_cpu >= 1)}"; then
    warn "CPU is moderate: avg ${avg_cpu}%"
else
    ok "CPU is fine: avg ${avg_cpu}%"
fi

if awk "BEGIN{exit !($avg_rss_mb >= 50)}"; then
    warn "Memory is HIGH: avg ${avg_rss_mb} MB  (expected < 20 MB)"
    heavy=1
else
    ok "Memory is fine: avg ${avg_rss_mb} MB"
fi

if [[ -n "$ps_ms" ]]; then
    if [[ $ps_ms -ge 100 ]]; then
        warn "ps scan is slow: ${ps_ms}ms  (this runs every refresh — consider raising refresh_ms in config)"
        heavy=1
    else
        ok "ps scan is fast: ${ps_ms}ms"
    fi
    if [[ $cap_ms -ge 50 ]]; then
        warn "capture-pane is slow: ${cap_ms}ms per session"
    else
        ok "capture-pane is fast: ${cap_ms}ms"
    fi
fi

echo ""
if [[ $heavy -eq 0 ]]; then
    echo -e "  ${GREEN}${BOLD}Overall: ccmux is lightweight ✓${RESET}"
else
    echo -e "  ${YELLOW}${BOLD}Overall: some overhead detected — see warnings above${RESET}"
fi
echo ""
