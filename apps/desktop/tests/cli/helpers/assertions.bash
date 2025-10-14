# Custom assertions for BATS

assert_file_exists() {
    if [ ! -f "$1" ]; then
        echo "Expected file to exist: $1" >&2
        return 1
    fi
}

assert_file_not_exists() {
    if [ -f "$1" ]; then
        echo "Expected file to not exist: $1" >&2
        return 1
    fi
}

assert_json_field() {
    local json=$1
    local field=$2
    local expected=$3

    # Check if jq is available
    if ! command -v jq &> /dev/null; then
        echo "jq is required for JSON assertions" >&2
        return 1
    fi

    local actual=$(echo "$json" | jq -r ".$field")
    if [ "$actual" != "$expected" ]; then
        echo "Expected $field=$expected, got: $actual" >&2
        return 1
    fi
}

assert_contains() {
    local haystack=$1
    local needle=$2

    if [[ "$haystack" != *"$needle"* ]]; then
        echo "Expected output to contain: $needle" >&2
        echo "Actual output: $haystack" >&2
        return 1
    fi
}

assert_not_contains() {
    local haystack=$1
    local needle=$2

    if [[ "$haystack" == *"$needle"* ]]; then
        echo "Expected output to NOT contain: $needle" >&2
        echo "Actual output: $haystack" >&2
        return 1
    fi
}

assert_process_running() {
    local pid=$1

    if ! ps -p $pid > /dev/null 2>&1; then
        echo "Expected process $pid to be running" >&2
        return 1
    fi
}

assert_process_not_running() {
    local pid=$1

    if ps -p $pid > /dev/null 2>&1; then
        echo "Expected process $pid to NOT be running" >&2
        return 1
    fi
}

assert_port_listening() {
    local port=$1

    if ! nc -z localhost $port 2>/dev/null; then
        echo "Expected port $port to be listening" >&2
        return 1
    fi
}

assert_port_not_listening() {
    local port=$1

    if nc -z localhost $port 2>/dev/null; then
        echo "Expected port $port to NOT be listening" >&2
        return 1
    fi
}
