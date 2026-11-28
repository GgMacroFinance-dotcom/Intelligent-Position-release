-- 每一轮工作记一行（backlog F-24，docs/ip/progress-design.md 2.1）：主机按同岗位的历史估任务进度和“要人看”的时间点。
-- 只是运行记录，不进事件日志；成功和失败的轮次都记。
CREATE TABLE turns (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    agent_id    TEXT NOT NULL,
    role_id     TEXT NOT NULL,
    thread_id   TEXT NOT NULL,
    started_ms  BIGINT NOT NULL,
    finished_ms BIGINT NOT NULL,
    steps       BIGINT NOT NULL,
    ok          BOOLEAN NOT NULL,
    cost_usd    DOUBLE PRECISION NOT NULL,
    error       TEXT
);
CREATE INDEX turns_by_thread ON turns (thread_id, started_ms);
CREATE INDEX turns_by_role ON turns (role_id, finished_ms);
CREATE INDEX turns_by_agent ON turns (agent_id, finished_ms);
CREATE INDEX turns_by_time ON turns (finished_ms);

-- 按对话找员工提问卡片（算“这个任务问过人没有、第几步问的”）。
CREATE INDEX approvals_by_subject_thread ON approvals ((json->'subject'->>'thread_id'));
