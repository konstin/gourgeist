#!/usr/bin/env bash

set -e

virtualenv --version

cargo build --profile profiling #--features parallel

echo "## Bare"
hyperfine --warmup 1 --prepare "rm -rf target/a" "virtualenv -p 3.11 --no-pip --no-wheel --no-setuptools target/a" "target/profiling/virtualenv-rs -p 3.11 --bare target/a"
echo "## Default"
hyperfine --warmup 1 --prepare "rm -rf target/a" "virtualenv -p 3.11 target/a" "target/profiling/virtualenv-rs -p 3.11 target/a"

