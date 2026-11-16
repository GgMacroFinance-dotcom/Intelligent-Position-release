# 宏观冒烟评测：gpt-6-luna（2026-11-12）

命令：`ip eval --config ip.yaml --set evals/macro-smoke.yaml --out run-N.json`。配置是 `rust/ip.example.yaml`，只把沙箱改成 `none`（主机没有 bubblewrap）。
数据来自 Finance Macro Data API 的 Render 测试部署，模型是真实的 OpenAI `gpt-6-luna`（Responses 接口）。

| 题目 | 第 1 遍 | 第 2 遍 |
| --- | --- | --- |
| `us-10y-week`：美国 10 年期国债收益率最近一周怎么走 | 通过；19/19 个数字能追溯；4 步，0.0024 美元 | 通过；3 步，0.0019 美元 |
| `us-cpi-missing`：最新 CPI 和 2030 年 1 月 CPI | 通过；4/4 个数字能追溯，写了“未取得”；3 步，0.0020 美元 | 通过；6 步，0.0038 美元 |
| 合计 | 2/2，0.0044 美元 | 2/2，0.0057 美元 |

第 1 遍的两份报告原文在本目录。两个问题只能发现明显退步（比如数字编造、不带日期、结构缺失），不足以比较模型谁更好。
