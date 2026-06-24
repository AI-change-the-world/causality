# Interactive Demo

运行前先启动服务端，然后执行：

```bash
python examples/openai_interactive_chat.py
```

如果服务地址不是 `http://127.0.0.1:8080`，先设置：

```bash
set MEMORY_SERVER_BASE_URL=http://127.0.0.1:8080
```

流程：

1. 输入 `system profile` 的自然语言介绍
2. 脚本会创建 profile 并确认 schema
3. 进入对话模式，普通输入会写入 event
4. 输入 `/ask 你的问题` 会触发检索，并输出 retrieval trace
5. 输入 `/quit` 退出
