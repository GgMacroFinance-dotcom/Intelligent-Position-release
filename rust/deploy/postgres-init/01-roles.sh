#!/bin/sh
# PostgreSQL 容器第一次启动（数据卷还是空的）时由官方镜像自动执行，之后不再执行。
# 建 I.P. 和团队上下文系统（TCMS）各自的登录角色和 schema，两边互不授权（docs/ip/postgres-migration-plan.md 第 3 节）。
# 密码从环境变量读：IP_DB_PASSWORD 必填；TCMS_DB_PASSWORD 没设时不建 tcms 那一份，以后要用再按本文件手工建。
set -eu

if [ -z "${IP_DB_PASSWORD:-}" ]; then
  echo "IP_DB_PASSWORD 没有设置" >&2
  exit 1
fi

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" \
  -v db="$POSTGRES_DB" -v ip_pw="$IP_DB_PASSWORD" <<'SQL'
-- 默认谁都能在 public 里建表、能连这个库；收回，只给两个应用角色连库的权限。
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
REVOKE ALL ON DATABASE :"db" FROM PUBLIC;

CREATE ROLE ip_app LOGIN PASSWORD :'ip_pw';
GRANT CONNECT ON DATABASE :"db" TO ip_app;
CREATE SCHEMA ip AUTHORIZATION ip_app;
ALTER ROLE ip_app SET search_path = ip;
SQL

if [ -n "${TCMS_DB_PASSWORD:-}" ]; then
  psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" \
    -v db="$POSTGRES_DB" -v tcms_pw="$TCMS_DB_PASSWORD" <<'SQL'
CREATE ROLE tcms_app LOGIN PASSWORD :'tcms_pw';
GRANT CONNECT ON DATABASE :"db" TO tcms_app;
CREATE SCHEMA tcms AUTHORIZATION tcms_app;
ALTER ROLE tcms_app SET search_path = tcms;
SQL
fi
