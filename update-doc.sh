#!/usr/bin/env bash

cargo doc --workspace --exclude example --no-deps
rm -rf docs
echo "<meta http-equiv=\"refresh\" content=\"0; url=gns\">" > target/doc/index.html
cp -r target/doc ./docs
