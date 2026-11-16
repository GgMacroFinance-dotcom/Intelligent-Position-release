# 运维测试岗位的演示项目

一个很小的“同比增速”脚本和它的测试，故意留了一个 bug，用来演示运维测试岗位的流程：
跑测试 → 定位原因 → 最小修改 → 复跑验证 → 写报告。

- `growth.sh`：`sh growth.sh <本期> <上年同期>`，输出同比增速（%）。
- `test.sh`：跑全部用例，全部通过时退出码为 0。

员工会直接改工作目录里的文件。演示前先复制一份，再把员工的 `workspace` 指过去：

```sh
cp -r examples/ops-demo /tmp/ops-demo
```

```yaml
agents:
  - { id: ops-1, role: ops-tester, name: 小维, workspace: /tmp/ops-demo }
```

然后对小维说“跑一下测试，有问题就修”。
