# syntax=docker/dockerfile:1
#
# intelligent-position harness 的镜像。用法见 docs/ip/deploy-docker.md。
#
# 在仓库根目录构建。宏观研究员的取数工具 macro-mcp/ 在 rust/ 之外，用命名构建上下文 macro-mcp 传进来
#（docker compose 会自动传，见 docker-compose.yml 的 additional_contexts）：
#
#   docker build --build-context macro-mcp=macro-mcp -t intelligent-position rust/
#   docker build --build-context macro-mcp=macro-mcp --build-arg INSTALL_CODEX=true -t intelligent-position rust/   # 带 Codex
#
# 镜像里的布局：/srv/ip 是工作目录，放着 ip.example.yaml 和 examples/；把自己的配置挂到 /srv/ip/ip.yaml。
# macro-mcp 在 /srv/macro-mcp，示例配置里的 ../macro-mcp/bin/whichapi-macro.js 从 /srv/ip 算起正好指到它。
# 配置里的相对路径以 /srv/ip 为准，所以 ip.example.yaml 的 data_dir: ./data、examples/… 不用改。
# /srv/ip/data（数据库、员工工作目录）和 /srv/ip/examples/public-data（ip data pull 拉下来的数据）要挂卷。
# 不要放到 /opt、/usr、/etc 下面：工具沙箱会把这三个目录只读挂给员工的 shell，
# 放在那里，每个员工都能读到数据库、ip.yaml 和别的员工的工作目录。

ARG RUST_VERSION=1.97
ARG DEBIAN_RELEASE=trixie

FROM rust:${RUST_VERSION}-slim-${DEBIAN_RELEASE} AS build
WORKDIR /src
COPY . .
# 依赖的下载和编译结果放在缓存里，只改源码时重新构建很快。
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p ip-cli \
 && cp target/release/ip /usr/local/bin/ip

FROM debian:${DEBIAN_RELEASE}-slim

# Node.js 一直装：macro-mcp 要用（18.17 以上，Debian trixie 自带的是 20）。
# 带 Codex 时再装 @openai/codex。CODEX_VERSION 可以固定到某个版本，比如 0.167.1。
# npm 只在安装 Codex 时用，装完就卸掉（连同它拉进来的依赖，约 150 MB）；Codex 运行时只要 node。
ARG INSTALL_CODEX=false
ARG CODEX_VERSION=latest

# bubblewrap：工具沙箱；git：员工和 Codex 在工作目录里用；tini：转发信号、回收子进程；nodejs：macro-mcp。
RUN apt-get update \
 && apt-get install -y --no-install-recommends bubblewrap ca-certificates git nodejs tini \
 && if [ "$INSTALL_CODEX" = "true" ]; then \
        apt-get install -y --no-install-recommends npm \
     && npm install -g "@openai/codex@${CODEX_VERSION}" \
     && npm cache clean --force \
     && apt-get purge -y --auto-remove npm; \
    fi \
 && rm -rf /var/lib/apt/lists/*

# 不用 root 运行。数据目录先建好并交给 ip 用户：挂上新的命名卷时，Docker 会连同属主一起初始化。
RUN useradd --create-home --home-dir /home/ip --uid 10001 --shell /bin/bash ip
COPY --from=build /usr/local/bin/ip /usr/local/bin/ip
COPY ip.example.yaml /srv/ip/ip.example.yaml
COPY examples /srv/ip/examples
# 只拷运行要的部分，不带测试。员工的 shell 在沙箱里看不到 /srv，这里只有代码，没有密钥。
COPY --from=macro-mcp bin /srv/macro-mcp/bin
COPY --from=macro-mcp src /srv/macro-mcp/src
COPY --from=macro-mcp package.json LICENSE README.md /srv/macro-mcp/
RUN mkdir -p /srv/ip/data && chown -R ip:ip /srv/ip/data /srv/ip/examples/public-data

USER ip
WORKDIR /srv/ip
VOLUME ["/srv/ip/data", "/srv/ip/examples/public-data"]
EXPOSE 7878

# 镜像里没有 curl，用 bash 自带的 /dev/tcp 请求 /healthz。
# 启动时要先连上所有 MCP 服务器才开始监听；本机的 macro-mcp 很快，但配了远程服务器、它又在休眠时要等一分钟左右，
# 所以起步期给到 150 秒。
# 起步期里第一次检查通过就算 healthy，不会多等。
HEALTHCHECK --interval=30s --timeout=5s --start-period=150s --retries=3 \
    CMD ["bash", "-c", "exec 3<>/dev/tcp/127.0.0.1/7878 && printf 'GET /healthz HTTP/1.0\\r\\n\\r\\n' >&3 && grep -q '\"status\":\"ok\"' <&3"]

ENTRYPOINT ["tini", "--", "ip"]
# 容器里要监听 0.0.0.0 才能映射端口；ip serve 要求这时配置里必须有 auth_token_env。
CMD ["serve", "--addr", "0.0.0.0:7878", "--config", "/srv/ip/ip.yaml"]
