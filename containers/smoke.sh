#!/bin/sh
set -eu
image=$1
work=$(mktemp -d)
cleanup() {
  if [ "$(id -u)" != 0 ] && [ "$(id -u)" != 65532 ]; then
    docker run --rm --entrypoint /bin/sh --user 0:0 --mount "type=bind,source=$work,target=/work" "$image" -c 'rm -rf /work/in' >/dev/null 2>&1 || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT
python3 fixtures/generate.py "$work/in"
chmod 755 "$work"
chmod -R a+rX,go-w "$work/in"
if [ "$(id -u)" != 0 ] && [ "$(id -u)" != 65532 ]; then
  docker run --rm --entrypoint /bin/sh --user 0:0 --mount "type=bind,source=$work,target=/work" "$image" -c 'chown -R 65532:65532 /work/in'
fi
run_restricted() {
  docker run --rm --network=none --read-only --cap-drop=ALL --security-opt=no-new-privileges --pids-limit=64 --memory=1g --cpus=1 --tmpfs /tmp:rw,nosuid,nodev,noexec,size=768m,mode=1777 --tmpfs /data/out:rw,nosuid,nodev,noexec,size=256m,uid=65532,gid=65532,mode=0700 --mount "type=bind,source=$work/in,target=/data/in,readonly" "$image" "$@"
}
run_restricted merge --root /data --request-json '{"inputs":[{"path":"in/three.pdf"},{"path":"in/one.pdf"}],"output":{"path":"out/merged.pdf"}}' | python3 -c 'import json,sys; v=json.load(sys.stdin); assert v["result"]["pageCount"]==4; print("non-root readonly network-none merge passed")'
run_restricted extract-text --root /data --request-json '{"input":{"path":"in/three.pdf"},"pages":[1]}' | python3 -c 'import json,sys; v=json.load(sys.stdin)["result"]; assert v["engine"]=="pdfium" and "PAGE-A" in v["pages"][0]["text"]; print("non-root readonly network-none PDFium text passed")'
run_restricted render-page --root /data --request-json '{"input":{"path":"in/three.pdf"},"page":1,"dpi":144,"output":{"path":"out/page.png"}}' | python3 -c 'import json,sys; v=json.load(sys.stdin)["result"]; assert v["engine"]=="pdfium" and (v["width"],v["height"])==(600,400) and v["bytes"]>0; print("non-root readonly network-none PDFium PNG 600x400 passed")'
if [ "${2:-}" = pdfoxide ]; then
  run_restricted extract-text --root /data --request-json '{"input":{"path":"in/one.pdf"},"pages":[1],"engine":"pdfoxide"}' | python3 -c 'import json,sys; v=json.load(sys.stdin)["result"]; assert v["engine"]=="pdfoxide" and v["pages"][0]["text"].strip()=="PAGE-ONE"; print("PDFOxide extraction passed")'
  run_restricted render-page --root /data --request-json '{"input":{"path":"in/one.pdf"},"page":1,"dpi":72,"engine":"pdfoxide","output":{"path":"out/one.png"}}' | python3 -c 'import json,sys; v=json.load(sys.stdin)["result"]; assert v["engine"]=="pdfoxide" and v["mediaType"]=="image/png"; print("PDFOxide rendering passed")'
fi
