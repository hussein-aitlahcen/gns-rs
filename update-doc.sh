#!/usr/bin/env bash

cargo doc --workspace --exclude example --no-deps
echo "<meta http-equiv=\"refresh\" content=\"0; url=gns\">" > target/doc/index.html
rm -rf docs
cp -r target/doc ./docs
