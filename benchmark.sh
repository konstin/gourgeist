#!/usr/bin/env bash

set -e

virtualenv --version

cargo build --profile profiling

echo "bare"
hyperfine --warmup 1 "virtualenv -p 3.11 --no-pip --no-wheel --no-setuptools target/a" "target/profiling/virtualenv-rs -p 3.11 --bare target/b"
echo "default"
hyperfine --warmup 1 "virtualenv -p 3.11 target/a" "target/profiling/virtualenv-rs -p 3.11 target/b"

