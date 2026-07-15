#!/usr/bin/env bash
# hot-path-write-audit / audit.sh
#
# Lint a single source file (*.rs / *.go / *.sql) for hot-path write anti-patterns.
# Exit code: 1 if any violation, 0 if clean.
#
# Patterns (matches the repository skill's "Hot-Path Write Complexity" rules):
#   1. wire-json-roundtrip       — serde_json::to_string(&payload|&wire|&msg|&event)
#   2. multi-clone-per-fn        — same function body has .clone() >= 2 times
#   3. rmw-array-column          — UPDATE ... SET (props|queue|list|history|mentions) = ...
#   4. json-array-default-column — schema:  props TEXT DEFAULT '[]'  /  queue TEXT DEFAULT '[]'
#   5. go-marshal-before-insert  — json.Marshal(payload|wire|msg|event) near INSERT/Exec
#
# Per-line ignore: append   // hot-path-audit: ignore   or   -- hot-path-audit: ignore
# Per-line cold-path waiver: append   // hot-path-audit: cold (≤1Hz, ≤128B, <reason>)

set -u

FILE="${1:-}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ -z "$FILE" ]; then
    echo "usage: $0 <path/to/file.{rs,go,sql}>" >&2
    exit 2
fi

if [ ! -f "$FILE" ]; then
    echo "error: file not found: $FILE" >&2
    exit 2
fi

# ANSI colors only when stdout is a tty
if [ -t 1 ]; then
    RED=$'\033[31m'
    GRAY=$'\033[90m'
    YELLOW=$'\033[33m'
    BOLD=$'\033[1m'
    RESET=$'\033[0m'
else
    RED=''
    GRAY=''
    YELLOW=''
    BOLD=''
    RESET=''
fi

EXT="${FILE##*.}"
COUNT_FILE="$(mktemp -t hot_path_audit.XXXXXX)"
echo 0 > "$COUNT_FILE"
trap 'rm -f "$COUNT_FILE"' EXIT

bump_counter() {
    local n
    n=$(cat "$COUNT_FILE")
    echo $((n + 1)) > "$COUNT_FILE"
}

emit() {
    # emit <pattern-name> <line-no> <line-content> <rule> <fix>
    local pat="$1" lno="$2" line="$3" rule="$4" fix="$5"
    printf '%s[hot-path-violation]%s %s:%s: %s%s%s\n' \
        "$RED$BOLD" "$RESET" "$FILE" "$lno" "$YELLOW" "$pat" "$RESET"
    printf '  %smatched:%s %s\n' "$GRAY" "$RESET" "$line"
    printf '  %srule:%s    %s\n' "$GRAY" "$RESET" "$rule"
    printf '  %sfix:%s     %s\n' "$GRAY" "$RESET" "$fix"
    bump_counter
}

# Skip lines containing the waiver markers
should_skip() {
    local line="$1"
    case "$line" in
        *"hot-path-audit: ignore"*) return 0 ;;
        *"hot-path-audit: cold"*)   return 0 ;;
    esac
    return 1
}

############################################
# Pattern 1: wire-json-roundtrip (Rust + Go)
############################################
check_wire_json_roundtrip() {
    # Rust: serde_json::to_string(&payload|&wire|&msg|&event|&e)
    # Rust: serde_json::to_vec(&...)
    # Also catches to_string_pretty.
    case "$EXT" in
        rs)
            grep -nE 'serde_json::(to_string(_pretty)?|to_vec)[[:space:]]*\([[:space:]]*&[[:space:]]*(payload|wire|msg|message|event|evt|m|wm|wire_msg|wire_message)' "$FILE" \
            | while IFS=: read -r lno line; do
                should_skip "$line" && continue
                emit "wire-json-roundtrip" "$lno" "${line# }" \
                    "禁止 serde_json::to_string(整条 wire 对象) 塞单列 — 协议字段无索引、热路径 CPU 浪费、schema 演进卡死" \
                    "把协议级稳定字段拆独立列；只有业务自由扩展才进 props (且独立列)。"
            done
            ;;
    esac
}

############################################
# Pattern 5: go-marshal-before-insert
############################################
check_go_marshal_before_insert() {
    case "$EXT" in
        go)
            # json.Marshal(payload|wire|msg|event|...)
            grep -nE 'json\.Marshal[[:space:]]*\([[:space:]]*&?[[:space:]]*(payload|wire|msg|message|event|evt|wm)' "$FILE" \
            | while IFS=: read -r lno line; do
                should_skip "$line" && continue
                # Look 1-15 lines ahead for INSERT / Exec / NamedExec / Query
                local follow
                follow=$(awk -v start="$lno" -v end="$((lno + 15))" 'NR>=start && NR<=end' "$FILE")
                if echo "$follow" | grep -qE '(INSERT[[:space:]]+INTO|\.Exec\(|\.NamedExec\(|\.QueryRow\(|\.NamedQuery\(|UPDATE[[:space:]]+)'; then
                    emit "go-marshal-before-insert" "$lno" "${line# }" \
                        "Go json.Marshal(整条 wire 对象) 紧邻 INSERT/Exec — 等价于 wire-json-roundtrip" \
                        "把协议级稳定字段拆独立列；写入时按列 bind，不要整体 marshal。"
                fi
            done
            ;;
    esac
}

############################################
# Pattern 3: rmw-array-column (UPDATE ... SET <jsoncol> = ...)
############################################
check_rmw_array_column() {
    # Catches: UPDATE foo SET props = ...
    # Targeted cols: props, queue, list, history, mentions, items, members_json
    grep -niE 'UPDATE[[:space:]]+[a-zA-Z_][a-zA-Z0-9_.]*[[:space:]]+SET[[:space:]]+(props|queue|list|history|mentions|items|members_json|tags_json|data_json)[[:space:]]*=' "$FILE" \
    | while IFS=: read -r lno line; do
        should_skip "$line" && continue
        emit "rmw-array-column" "$lno" "${line# }" \
            "UPDATE 整列回写 JSON 数组 = 读出 → 应用层 push → 写回 = O(N) per event，规模翻倍即崩" \
            "拆成独立子表 + 复合主键 (parent_id, item_id)；append=O(1) INSERT，清理=DELETE WHERE。"
    done
}

############################################
# Pattern 4: json-array-default-column (schema)
############################################
check_json_array_default_column() {
    # props TEXT DEFAULT '[]'
    # queue TEXT NOT NULL DEFAULT '[]'
    # mentions TEXT DEFAULT '[]'
    grep -niE '(props|queue|list|history|mentions|items|members_json|tags_json|data_json)[[:space:]]+(TEXT|VARCHAR|JSON|JSONB|BLOB)[^,]*DEFAULT[[:space:]]+'\''(\[\]|\{\})'\' "$FILE" \
    | while IFS=: read -r lno line; do
        should_skip "$line" && continue
        emit "json-array-default-column" "$lno" "${line# }" \
            "schema 里 JSON 数组列 (DEFAULT '[]') 注定走 read-modify-write 热路径，违反铁律 #4" \
            "改 normalized 子表：CREATE TABLE parent_item(parent_id, item_id, ..., PRIMARY KEY(parent_id, item_id))。"
    done
}

############################################
# Pattern 2: multi-clone-per-fn (Rust + Go via copies)
############################################
# Use awk to walk the file, track function boundaries, count .clone() per fn.
# - Rust fn boundary:   /^[[:space:]]*(pub[[:space:]]+)?(async[[:space:]]+)?fn[[:space:]]+/  start
#                       brace-depth back to 0 → end
# - Go func boundary:   /^func[[:space:]]+/  start; brace-depth back to 0 → end
# Heuristic brace counting works for well-formatted source; good enough for lint.
check_multi_clone_per_fn() {
    case "$EXT" in
        rs|go) ;;
        *) return ;;
    esac

    local clone_marker
    clone_marker="$(mktemp -t hot_path_clone.XXXXXX)"
    awk -v ext="$EXT" -v file="$FILE" -v red="$RED" -v bold="$BOLD" -v gray="$GRAY" -v yellow="$YELLOW" -v reset="$RESET" -v marker="$clone_marker" '
    function reset_fn() {
        in_fn = 0
        fn_name = ""
        fn_start_line = 0
        depth = 0
        clone_count = 0
        delete clone_lines
        delete clone_texts
    }
    BEGIN { reset_fn() }
    {
        line = $0
        # Detect function start when not already inside one
        if (!in_fn) {
            if (ext == "rs" && match(line, /^[[:space:]]*(pub[[:space:]]+)?(async[[:space:]]+)?fn[[:space:]]+[A-Za-z_][A-Za-z0-9_]*/)) {
                in_fn = 1
                fn_start_line = NR
                # extract name
                tmp = line
                sub(/^[[:space:]]*(pub[[:space:]]+)?(async[[:space:]]+)?fn[[:space:]]+/, "", tmp)
                match(tmp, /[A-Za-z_][A-Za-z0-9_]*/)
                fn_name = substr(tmp, RSTART, RLENGTH)
            } else if (ext == "go" && match(line, /^func[[:space:]]/)) {
                in_fn = 1
                fn_start_line = NR
                tmp = line
                # func (recv) Name(...)  or  func Name(...)
                sub(/^func[[:space:]]+/, "", tmp)
                sub(/^\([^)]*\)[[:space:]]*/, "", tmp)
                match(tmp, /[A-Za-z_][A-Za-z0-9_]*/)
                fn_name = substr(tmp, RSTART, RLENGTH)
            }
        }

        if (in_fn) {
            # update brace depth
            n_open = gsub(/\{/, "{", line)
            # gsub returns count; we used the same string both sides so line is preserved
            n_close = gsub(/\}/, "}", line)
            depth += n_open - n_close

            # count .clone() occurrences on this line (skip if waiver present)
            if (index(line, "hot-path-audit: ignore") == 0 && index(line, "hot-path-audit: cold") == 0) {
                # count occurrences of .clone(
                tmpline = line
                local_count = 0
                while (match(tmpline, /\.clone\(/)) {
                    local_count++
                    tmpline = substr(tmpline, RSTART + RLENGTH)
                }
                if (local_count > 0) {
                    clone_count += local_count
                    clone_lines[length(clone_lines) + 1] = NR
                    clone_texts[length(clone_texts) + 1] = line
                }
            }

            # function end: depth back to 0 after we saw at least one {
            if (depth <= 0 && (n_open > 0 || NR > fn_start_line)) {
                if (clone_count >= 2) {
                    printf "%s[hot-path-violation]%s %s:%s: %smulti-clone-per-fn%s\n", red bold, reset, file, clone_lines[1], yellow, reset
                    for (i = 1; i <= length(clone_lines); i++) {
                        gsub(/^[[:space:]]+/, "", clone_texts[i])
                        printf "  %smatched:%s %s:%s  %s\n", gray, reset, fn_name, clone_lines[i], clone_texts[i]
                    }
                    printf "  %srule:%s    一个函数体出现 ≥ 2 次 .clone() (实际 %d 次) — 热路径深拷贝堆叠，违反 O(1) 铁律\n", gray, reset, clone_count
                    printf "  %sfix:%s     用借用 / Arc / 字段拆分；只在出参 boundary 处必要时 clone 一次\n", gray, reset
                    print "V" >> marker
                }
                reset_fn()
            }
        }
    }
    ' "$FILE"

    if [ -s "$clone_marker" ]; then
        local extra
        extra=$(wc -l < "$clone_marker" | tr -d ' ')
        local cur
        cur=$(cat "$COUNT_FILE")
        echo $((cur + extra)) > "$COUNT_FILE"
    fi
    rm -f "$clone_marker"
}

############################################
# Run all checks
############################################

case "$EXT" in
    rs)
        check_wire_json_roundtrip
        check_rmw_array_column
        check_multi_clone_per_fn
        ;;
    go)
        check_go_marshal_before_insert
        check_rmw_array_column
        check_multi_clone_per_fn
        ;;
    sql)
        check_json_array_default_column
        check_rmw_array_column
        ;;
    *)
        echo "warn: unsupported extension '.$EXT' — supported: .rs .go .sql" >&2
        exit 2
        ;;
esac

# Also run schema check on rs/go because folks embed SQL in source
case "$EXT" in
    rs|go)
        check_json_array_default_column
        ;;
esac

VIOLATION_COUNT=$(cat "$COUNT_FILE")
if [ "$VIOLATION_COUNT" -gt 0 ]; then
    printf '\n%s%d violation(s)%s in %s — see %s/SKILL.md\n' \
        "$RED$BOLD" "$VIOLATION_COUNT" "$RESET" "$FILE" "$SCRIPT_DIR"
    exit 1
fi

printf '%sno hot-path violations%s in %s\n' "${GRAY}" "${RESET}" "$FILE"
exit 0
