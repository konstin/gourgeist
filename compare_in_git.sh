#!/usr/bin/env bash

set -e

rm -rf compare_venv
virtualenv -p 3.11 compare_venv
rm compare_venv/.gitignore
git -C compare_venv init
git -C compare_venv add -A
git -C compare_venv commit -q -m "Initial commit"
rm -r compare_venv/* # This skips the hidden .git
mv compare_venv compare_venv2
cargo run -- compare_venv
rm compare_venv/.gitignore
cp -r compare_venv/* compare_venv2
rm -r compare_venv
mv compare_venv2 compare_venv
git -C compare_venv/ status
