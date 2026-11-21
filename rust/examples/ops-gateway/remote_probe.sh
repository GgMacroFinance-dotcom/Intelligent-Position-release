#!/bin/sh
# 在中转站主机上跑的只读探针：容器健康、系统服务、磁盘、最近一次数据库备份。
# 不改任何东西，不读密钥文件。每行输出一个 JSON 对象，由 gateway_check.py 汇总。
#
# 用法（gateway_check.py 会替你调用）：
#   ssh <主机> 'sudo -n sh -s' < remote_probe.sh
# 要调整的东西用 PROBE_ 开头的变量传；gateway_check.py 的 --remote-env 会把它们写在脚本开头：
#   PROBE_COMPOSE_PROJECT  docker compose 的项目名，默认 new-api
#   PROBE_SERVICES         要求 active 的 systemd 服务，空格分隔，默认 nginx
#   PROBE_DISK_PATHS       要看用量的挂载点，空格分隔，默认 /
#   PROBE_BACKUP_DIR       数据库备份目录；不设就不查备份
#   PROBE_BACKUP_GLOB      备份文件名的通配，默认 *.dump
set -u

project=${PROBE_COMPOSE_PROJECT:-new-api}
services=${PROBE_SERVICES:-nginx}
disks=${PROBE_DISK_PATHS:-/}
backup_dir=${PROBE_BACKUP_DIR:-}
backup_glob=${PROBE_BACKUP_GLOB:-*.dump}

# JSON 字符串转义（只处理会出现的反斜杠和双引号）。
q() { printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'; }

echo "{\"kind\":\"host\",\"name\":\"$(q "$(hostname)")\",\"now_s\":$(date +%s)}"

# 容器：名字和 docker 的状态文字，比如 “Up 3 days (healthy)”。
if command -v docker >/dev/null 2>&1; then
  docker ps -a --filter "label=com.docker.compose.project=$project" --format '{{.Names}}	{{.Status}}' 2>/dev/null |
    while IFS='	' read -r name status; do
      echo "{\"kind\":\"container\",\"name\":\"$(q "$name")\",\"status\":\"$(q "$status")\"}"
    done
else
  echo '{"kind":"error","what":"docker","detail":"没有 docker 命令"}'
fi

for s in $services; do
  state=$(systemctl is-active "$s" 2>/dev/null || true)
  echo "{\"kind\":\"service\",\"name\":\"$(q "$s")\",\"state\":\"$(q "${state:-unknown}")\"}"
done

for d in $disks; do
  # df -P 第二行：文件系统 总块 已用 可用 用量% 挂载点
  pct=$(df -P "$d" 2>/dev/null | awk 'NR==2 {gsub("%","",$5); print $5}')
  echo "{\"kind\":\"disk\",\"path\":\"$(q "$d")\",\"used_pct\":${pct:-null}}"
done

if [ -n "$backup_dir" ]; then
  # 最新的一个备份文件：修改时间（秒）和大小。
  latest=$(find "$backup_dir" -maxdepth 1 -type f -name "$backup_glob" -printf '%T@ %s %f\n' 2>/dev/null | sort -n | tail -1)
  if [ -n "$latest" ]; then
    set -- $latest
    echo "{\"kind\":\"backup\",\"file\":\"$(q "$3")\",\"mtime_s\":${1%.*},\"bytes\":$2}"
  else
    echo "{\"kind\":\"backup\",\"file\":null}"
  fi
fi
