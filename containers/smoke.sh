#!/bin/sh
set -eu
image=$1
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
python3 fixtures/generate.py "$work/in"
chmod 755 "$work"
chmod -R a+rX "$work/in"
docker run --rm --network=none --read-only --cap-drop=ALL --security-opt=no-new-privileges --pids-limit=64 --memory=1g --cpus=1 --tmpfs /tmp:rw,nosuid,nodev,noexec,size=768m,mode=1777 --tmpfs /data/out:rw,nosuid,nodev,noexec,size=256m,uid=65532,gid=65532,mode=0700 --mount "type=bind,source=$work/in,target=/data/in,readonly" "$image" merge --root /data --request-json '{"inputs":[{"path":"in/three.pdf"},{"path":"in/one.pdf"}],"output":{"path":"out/merged.pdf"}}' | python3 -c 'import json,sys; v=json.load(sys.stdin); assert v["result"]["pageCount"]==4; print("non-root readonly network-none merge passed")'
