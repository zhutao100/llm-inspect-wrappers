#!/usr/bin/env bash
# xwrap — LLM-agentic thin wrappers for fd, rg, sed
# Install: place as "xwrap" on PATH, then:
#   ln -s xwrap fd-x && ln -s xwrap rg-x && ln -s xwrap sed-x
#
# Design: best-effort post-process, passthrough on any failure.

set -o pipefail

# ---------------------------------------------------------------------------
# Tunables
# ---------------------------------------------------------------------------
# Max line length (chars) before a line is considered "huge" and gets gated
HUGE_LINE_THRESHOLD="${XWRAP_HUGE_LINE:-1000}"
# How many chars to show from head/tail of a gated line
GATE_PEEK="${XWRAP_GATE_PEEK:-120}"
# Max number of files to annotate with size/LoC (guard against runaway)
MAX_ANNOTATE="${XWRAP_MAX_ANNOTATE:-500}"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# Annotate a single file path with byte-size and line-count.
# Output: "path [size LoC]" or just "path" on failure.
# Uses wc -cl which is POSIX and fast (single syscall per file).
_annotate_file() {
    local f="$1"
    if [[ ! -f "$f" ]]; then
        printf '%s\n' "$f"
        return
    fi
    # wc -c -l: lines then bytes (macOS wc outputs " lines  bytes filename")
    local wc_out
    wc_out=$(wc -c -l < "$f" 2>/dev/null) || { printf '%s\n' "$f"; return; }
    # wc < file outputs "  lines  bytes" (no filename)
    local lines bytes
    read -r lines bytes <<< "$wc_out"
    printf '%s  [%s bytes, %s lines]\n' "$f" "$bytes" "$lines"
}

# Gate a single line: if over threshold, replace with truncated preview + hint.
_gate_line() {
    local line="$1"
    local len=${#line}
    if (( len <= HUGE_LINE_THRESHOLD )); then
        printf '%s\n' "$line"
        return
    fi
    # Build a heuristic hint about the content
    local hint=""
    # Detect JSON-ish
    if [[ "$line" =~ ^\{.*\}$ ]] || [[ "$line" =~ ^\[.*\]$ ]]; then
        # Try to extract top-level keys (best-effort, no jq dependency)
        # Grab first few "key": patterns
        local keys
        keys=$(printf '%s' "$line" | grep -oE '"[a-zA-Z_][a-zA-Z0-9_]{0,30}"[[:space:]]*:' | head -6 | tr -d '"' | tr -d ':' | tr '\n' ',' | sed 's/,$//')
        if [[ -n "$keys" ]]; then
            hint="json object, keys: [${keys}]"
        else
            hint="json blob"
        fi
    elif [[ "$line" =~ ^[[:space:]]*\<[a-zA-Z] ]]; then
        hint="xml/html markup"
    elif [[ "$line" =~ base64 ]] || [[ "$line" =~ ^[A-Za-z0-9+/=]{200} ]]; then
        hint="likely base64 data"
    else
        hint="long text"
    fi

    local head_part="${line:0:$GATE_PEEK}"
    local tail_part="${line: -$GATE_PEEK}"
    printf '%s  ...⟪TRUNCATED %d chars, %s⟫...  %s\n' "$head_part" "$len" "$hint" "$tail_part"
}

# Collect unique file paths from stdin (one per line), annotate them,
# and print a deduplicated trailer block.
# Reads paths from fd "$@" array if provided, otherwise from lines on stdin.
_print_file_annotations() {
    local -a paths=("$@")
    local count=${#paths[@]}
    if (( count == 0 )); then
        return
    fi
    if (( count > MAX_ANNOTATE )); then
        printf '\n--- file info (showing %d of %d files, truncated) ---\n' "$MAX_ANNOTATE" "$count"
        paths=("${paths[@]:0:$MAX_ANNOTATE}")
    else
        printf '\n--- file info (%d files) ---\n' "$count"
    fi
    for f in "${paths[@]}"; do
        _annotate_file "$f"
    done
}

# ---------------------------------------------------------------------------
# fd-x: pass-through fd, then annotate result paths with size/LoC
# ---------------------------------------------------------------------------
_fdx() {
    # Run fd with original args, capture output
    local output
    output=$(fd "$@" 2>&1) || {
        # On failure, passthrough whatever fd produced (errors, empty, etc.)
        printf '%s\n' "$output"
        return $?
    }

    if [[ -z "$output" ]]; then
        return 0
    fi

    # Collect paths (fd outputs one per line by default when piped)
    local -a all_paths=()
    while IFS= read -r line; do
        all_paths+=("$line")
    done <<< "$output"

    # Print original fd output verbatim
    printf '%s\n' "$output"

    # Annotate only regular files (skip dirs unless user wants)
    local -a file_paths=()
    for p in "${all_paths[@]}"; do
        if [[ -f "$p" ]]; then
            file_paths+=("$p")
        fi
    done

    if (( ${#file_paths[@]} > 0 )); then
        _print_file_annotations "${file_paths[@]}"
    fi
}

# ---------------------------------------------------------------------------
# rg-x: pass-through rg, annotate matched files, gate huge match lines
# ---------------------------------------------------------------------------
_rgx() {
    # Detect if user is using modes where output is just file paths
    local files_mode=0
    for arg in "$@"; do
        case "$arg" in
            -l|--files-with-matches|--files-without-match|--files)
                files_mode=1 ;;
        esac
    done

    # Run rg, capture raw output and exit code
    local rc=0
    local output
    output=$(rg --no-heading --line-number "$@" 2>&1) || rc=$?

    # Exit code 1 = no matches (normal), 2 = error. On error, passthrough.
    if (( rc == 2 )); then
        printf '%s\n' "$output"
        return "$rc"
    fi

    if [[ -z "$output" ]]; then
        return "$rc"
    fi

    # In files-only mode: output is one path per line, annotate and done
    if (( files_mode )); then
        printf '%s\n' "$output"
        local -a fpaths=()
        while IFS= read -r line; do
            [[ -n "$line" ]] && fpaths+=("$line")
        done <<< "$output"
        _print_file_annotations "${fpaths[@]}"
        return "$rc"
    fi

    # Match-content mode: parse "path:line:text" lines.
    # - Gate huge lines
    # - Collect unique file paths for annotation trailer
    local -A seen_files=()
    local -a unique_files=()

    while IFS= read -r line; do
        # rg --no-heading output: path:linenum:content  or  path:linenum-content (context)
        # Also possible: path-linenum-content for context lines with -C
        # We gate the *displayed* line content regardless.
        local len=${#line}
        if (( len > HUGE_LINE_THRESHOLD )); then
            # Extract the path:linenum: prefix before gating content
            # Regex: everything up to the second colon cluster is prefix
            if [[ "$line" =~ ^([^:]+:[0-9]+:) ]]; then
                local prefix="${BASH_REMATCH[1]}"
                local content="${line:${#prefix}}"
                printf '%s' "$prefix"
                _gate_line "$content"
            else
                _gate_line "$line"
            fi
        else
            printf '%s\n' "$line"
        fi

        # Extract file path for annotation (first colon-delimited field)
        local fpath
        fpath="${line%%:*}"
        if [[ -f "$fpath" ]] && [[ -z "${seen_files[$fpath]+_}" ]]; then
            seen_files[$fpath]=1
            unique_files+=("$fpath")
        fi
    done <<< "$output"

    if (( ${#unique_files[@]} > 0 )); then
        _print_file_annotations "${unique_files[@]}"
    fi

    return "$rc"
}

# ---------------------------------------------------------------------------
# sed-x: gate ranged reads, annotate file size/LoC
# ---------------------------------------------------------------------------
_sedx() {
    # Detect the specific pattern: sed -n 'START,ENDp' FILE [FILE...]
    # If this is not a ranged read, passthrough entirely.
    local is_ranged=0
    local nflag=0
    local range_expr=""
    local -a files=()
    local -a all_args=("$@")

    # Simple arg scan: look for -n and a 'N,Mp' expression
    local i=0
    local expect_expr_next=0
    for arg in "$@"; do
        if [[ "$arg" == "-n" ]]; then
            nflag=1
        elif (( nflag )) && [[ "$arg" =~ ^([0-9]+),([0-9]+)p$ ]]; then
            is_ranged=1
            range_expr="$arg"
        elif (( nflag )) && [[ "$arg" =~ ^\'?([0-9]+),([0-9]+)p\'?$ ]]; then
            is_ranged=1
            range_expr="$arg"
        elif (( is_ranged )) && [[ -e "$arg" ]]; then
            files+=("$arg")
        fi
    done

    # Not a ranged read — full passthrough
    if (( ! is_ranged )); then
        exec sed "$@"
    fi

    # Run sed with original args
    local rc=0
    local output
    output=$(sed "$@" 2>&1) || rc=$?

    if (( rc != 0 )); then
        printf '%s\n' "$output"
        return "$rc"
    fi

    # Post-process: gate huge lines
    local had_gated=0
    while IFS= read -r line; do
        local len=${#line}
        if (( len > HUGE_LINE_THRESHOLD )); then
            _gate_line "$line"
            had_gated=1
        else
            printf '%s\n' "$line"
        fi
    done <<< "$output"

    if (( had_gated )); then
        printf '\n⚠  WARNING: some lines were truncated (>%d chars). Use targeted extraction if full content needed.\n' "$HUGE_LINE_THRESHOLD"
    fi

    # Append file info trailer
    for f in "${files[@]}"; do
        if [[ -f "$f" ]]; then
            local wc_out
            wc_out=$(wc -c -l < "$f" 2>/dev/null) || continue
            local lines bytes
            read -r lines bytes <<< "$wc_out"
            printf '\n--- %s: %s bytes, %s lines ---\n' "$f" "$bytes" "$lines"
        fi
    done

    return "$rc"
}

# ---------------------------------------------------------------------------
# Dispatch based on argv[0] basename
# ---------------------------------------------------------------------------
_main() {
    local progname
    progname=$(basename "$0")

    case "$progname" in
        fd-x)   _fdx "$@" ;;
        rg-x)   _rgx "$@" ;;
        sed-x)  _sedx "$@" ;;
        xwrap)
            # Direct invocation: first arg is the sub-command
            local subcmd="${1:-}"
            shift 2>/dev/null || true
            case "$subcmd" in
                fd)   _fdx "$@" ;;
                rg)   _rgx "$@" ;;
                sed)  _sedx "$@" ;;
                *)
                    echo "usage: xwrap {fd|rg|sed} [args...]" >&2
                    echo "   or: symlink as fd-x / rg-x / sed-x" >&2
                    return 1
                    ;;
            esac
            ;;
        *)
            # Unknown personality — full passthrough to avoid breaking anything
            echo "xwrap: unknown invocation '$progname', passing through" >&2
            exec "$progname" "$@"
            ;;
    esac
}

_main "$@"
