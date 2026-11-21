-- 演练用的 New API logs 表（只建 usage_report.py 用到的列）和一天的假日志。
-- 时间用 :DAY_START 占位，drill.sh 换成当天 00:00（+08:00）的 Unix 秒。
CREATE TABLE logs (user_id INT, created_at BIGINT, type INT, content TEXT, username TEXT,
  model_name TEXT, quota INT, prompt_tokens INT, completion_tokens INT, use_time INT);
INSERT INTO logs (user_id, created_at, type, content, username, model_name, quota, prompt_tokens, completion_tokens, use_time) VALUES
 (1, :DAY_START + 3600,  2, '', 'alice', 'model-a', 250000, 1200, 300, 4),
 (1, :DAY_START + 3700,  2, '', 'alice', 'model-a', 250000, 1000, 500, 6),
 (2, :DAY_START + 36000, 2, '', 'bob',   'model-b', 100000, 800,  200, 2),
 (2, :DAY_START + 36100, 5, 'upstream authentication failed (401)|渠道 1', 'bob', 'model-b', 0, 0, 0, 0),
 (3, :DAY_START + 50000, 5, 'A timeout occurred
524', 'carol', 'model-a', 0, 0, 0, 0),
 (3, :DAY_START + 50100, 5, 'A timeout occurred
524', 'carol', 'model-a', 0, 0, 0, 0),
 (3, :DAY_START - 100,   2, '', 'carol', 'model-a', 999999, 9, 9, 9),
 (4, :DAY_START + 60000, 1, '充值', 'dave', '', 5000000, 0, 0, 0);
