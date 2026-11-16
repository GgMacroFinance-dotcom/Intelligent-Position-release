#!/usr/bin/env bash
# 检查一段提交里每个非合并提交都有 Signed-off-by 一行（DCO，见根目录 CONTRIBUTING.md）。
# 用法：check-dco.sh <base> <head>
set -euo pipefail
base="$1"
head="$2"
missing=0
for sha in $(git rev-list --no-merges "${base}..${head}"); do
  if ! git log -1 --format=%B "$sha" | grep -Eq '^Signed-off-by: .+ <[^<>@ ]+@[^<>@ ]+>$'; then
    echo "缺少 Signed-off-by：$(git log -1 --format='%h %s' "$sha")"
    missing=1
  fi
done
if [ "$missing" -ne 0 ]; then
  echo
  echo "每个提交都要带 Signed-off-by 一行。新提交用 git commit -s；已有的提交用 git rebase --signoff <base> 补上。"
  exit 1
fi
echo "全部提交都有 Signed-off-by。"
