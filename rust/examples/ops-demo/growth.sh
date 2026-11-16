#!/bin/sh
# 用法：sh growth.sh <本期> <上年同期>，输出同比增速（%），保留一位小数。
# 上年同期为 0 时无法计算，输出 NA。
awk -v c="$1" -v p="$2" 'BEGIN {
  if (p == 0) { print "NA"; exit }
  printf "%.1f\n", (c - p) / c * 100
}'
