#!/bin/sh
# 跑全部用例：sh test.sh。全部通过时退出码为 0。
fail=0
check() {
  got=$(sh growth.sh "$1" "$2")
  if [ "$got" = "$3" ]; then
    echo "ok   growth($1, $2) = $got"
  else
    echo "FAIL growth($1, $2) = $got，应为 $3"
    fail=1
  fi
}
check 102 100 2.0
check 110 100 10.0
check 90 100 -10.0
check 5 0 NA
exit $fail
