-- 对话列表带上“最后由哪位员工回复”（backlog F-23 ⑤）：App 重启后，没挂在任务、审批上的普通对话也能归到员工名下。
-- 写入时由 writer.rs 的 append_message 更新；已有的对话从 messages 里按写入顺序补一次。
ALTER TABLE threads ADD COLUMN last_agent_id TEXT;

UPDATE threads SET last_agent_id = latest.agent_id
FROM (
    SELECT DISTINCT ON (thread_id) thread_id, json->'author'->>'agent_id' AS agent_id
    FROM messages
    WHERE json->'author'->>'kind' = 'agent'
    ORDER BY thread_id, pos DESC
) AS latest
WHERE latest.thread_id = threads.id;
