#!/bin/bash
# Copyright 2020-2021 The Datafuse Authors.
# SPDX-License-Identifier: Apache-2.0.


get_latest_tag() {
  curl --silent "https://api.github.com/repos/$1/tags" | # Get latest release from GitHub api
    grep '"name":' |                                            # Get tag line
    sed -E 's/.*"([^"]+)".*/\1/' | grep 'v' | head -1
}

tag=`get_latest_tag "datafuselabs/datafuse"`


SCRIPT_PATH="$( cd "$( dirname "$0" )" >/dev/null 2>&1 && pwd )"
cd "$SCRIPT_PATH/../.." || exit

killall fuse-store
killall fuse-query
sleep 1

wget --quiet -O target/fusequery-${tag}-linux-x86_64 "https://github.com/datafuselabs/datafuse/releases/download/${tag}/fusequery-${tag}-linux-x86_64"
chmod +x target/fusequery-${tag}-linux-x86_64

echo 'Start FuseQuery...'
nohup ./target/fusequery-${tag}-linux-x86_64 -c scripts/deploy/config/fusequery-node-1.toml &
echo "Waiting on fuse-query 10 seconds..."
timeout 10 sh -c 'until nc -z $0 $1; do sleep 1; done' 0.0.0.0 3307

