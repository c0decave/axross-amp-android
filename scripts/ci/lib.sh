# shellcheck shell=bash
# Shared by the scripts in scripts/ci/ (sourced, not run).
#
# step NAME COMMAND...  runs COMMAND in a folded log group and keeps its
# output; when it fails, the lines that say why and the end of its output
# become error annotations of the job. Annotations can be read through
# GitHub's API (check runs) where a run's full log cannot be fetched.

CI_LOGS=${CI_LOGS:-${RUNNER_TEMP:-/tmp}/ci-logs}
mkdir -p "$CI_LOGS"

# The text as a workflow command's message: one line, % \r \n escaped, no
# terminal colours.
ci_escape() {
    local text
    text=$(sed -e 's/\x1b\[[0-9;]*[A-Za-z]//g' -e 's/\r$//')
    text=${text//'%'/'%25'}
    text=${text//$'\r'/'%0D'}
    text=${text//$'\n'/'%0A'}
    printf '%s' "$text"
}

# annotate LEVEL TITLE < text   (LEVEL: error, warning or notice)
annotate() {
    local level=$1 title=$2 body
    body=$(ci_escape)
    title=${title//'%'/'%25'}
    title=${title//,/'%2C'}
    title=${title//:/'%3A'}
    [ -n "$body" ] || return 0
    printf '::%s title=%s::%s\n' "$level" "$title" "$body"
}

# The lines of a log that usually say why a build failed.
why_lines() {
    sed -e 's/\x1b\[[0-9;]*[A-Za-z]//g' "$1" \
        | grep -E -A3 '(^|[^a-z])(error|ERROR|FAILURE|FAILED|What went wrong|Exception|Could not|could not|not found|No such file|Caused by|undefined reference|cannot find)' \
        | grep -v -E '^\s*at (org\.gradle|java\.|jdk\.|sun\.|kotlin\.|groovy\.)' \
        | head -n 70 || true
}

# (call it as a statement, not as `step ... || ...`: bash ignores errexit
# inside a command whose status is tested, and a function given to step
# would then go on past a failing line)
step() {
    local name=$1 log rc
    shift
    log="$CI_LOGS/$(printf '%s' "$name" | tr -c 'A-Za-z0-9' '_').log"
    echo "::group::$name"
    echo "+ $*"
    set +e
    # (errexit inside: a function stops at its first failing command)
    (set -e; "$@") 2>&1 | tee "$log"
    rc=${PIPESTATUS[0]}
    set -e
    echo "::endgroup::"
    if [ "$rc" -ne 0 ]; then
        # (no error status from the reporting itself)
        set +e
        why_lines "$log" | annotate error "$name failed (exit $rc) - why"
        tail -n 60 "$log" | annotate error "$name failed (exit $rc) - the end of its output"
        echo "$name failed (exit $rc)"
        set -e
        return "$rc"
    fi
}

# gradle ARGS... with retries: Maven Central answers 429 (too many
# requests) now and then to a runner that resolves the Android Gradle
# Plugin's tree - waited for (1, 2, 3 minutes) and tried again; any other
# failure is one at once.
gradle_retrying() {
    local try log="$CI_LOGS/gradle-try.log"
    for try in 1 2 3 4; do
        if gradle --no-daemon --console=plain --stacktrace "$@" 2>&1 | tee "$log"; then
            return 0
        fi
        if [ "$try" -eq 4 ] || ! grep -q -E "status code 429|Too Many Requests|Could not GET 'https://" "$log"; then
            return 1
        fi
        echo "a Maven repository refused (429 or unreachable): once more in $((try * 60)) s"
        sleep $((try * 60))
    done
}
