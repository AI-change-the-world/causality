import requests
import json

url = "http://localhost:18081/api/v1/events"

__event__ = """
用户：  
最近在整理代码仓库，发现自己真的越来越不想碰 C++ 了。  
不是说它不好，就是那种“我知道我能写，但我不想维护”的感觉。

前两天顺手又写了个 Rust 小工具，用来扫日志和做一点聚合，
Cargo 一路拉依赖、跑测试，非常顺。
写完以后我甚至懒得给 C++ 那个版本补文档。

不过说起来，昨天晚上没怎么写代码。
我在写一个小说片段，大概是一个在废弃空间站里的工程师，
他在修一个永远也修不好的生命维持系统。
写着写着就觉得挺像现实的，系统复杂到没人完全理解。

小说里有一段是这样的：
“他知道，只要再加一个补丁，系统还能撑三个月。
但三个月之后，还是会有人站在同样的位置，看同样的日志。”

写到这里我突然意识到，
我其实挺讨厌那种只是在不断打补丁、没有结构改进的工作。
这也是我不太喜欢纯 CRUD 项目的原因，
每天都是接口、表、字段，对我来说太消耗耐心了。

当然，有时候我也会怀疑是不是自己太挑了。
毕竟活总要有人干。
但如果可以选，我还是更愿意做偏架构、偏工具、偏抽象的事情。

对了，说个完全无关的，
最近咖啡喝多了，晚上有点睡不着，
可能得控制一下摄入量。

回到技术上，
如果非要我在“性能极致但难维护”和“性能够用但清晰安全”之间选，
我大概率会选后者。
所以即便 Rust 在某些场景下慢一点，我也能接受。

管理方向我暂时没太大兴趣，
比起开会和对人负责，我更享受对系统负责。

——
以上基本就是一些零碎的想法，
不一定都重要，可能明天我自己再看都会觉得有点啰嗦。

"""

payload = {
    "owner_id": "user_123",
    "content": __event__,
    "llm_provider": "qwen",
    "embedding_provider":"ebd",
    "context": None,
    "scope_id": "global-1-2-3-4",
    "source": "diary"
}

# payload = {
#     "owner_id": "user_123",
#     "content": "This is a test content for memory extraction with enough characters",
#     "llm_provider": "qwen",
#     "embedding_provider": "ebd",
#     "context": None,
#     "scope_id": "global-1-2-3-4",
#     "source": "diary"
# }


headers = {
    "Content-Type": "application/json",
    # "Authorization": "Bearer YOUR_TOKEN"
}

resp = requests.post(url, json=payload, headers=headers, timeout=3600)

print(resp.status_code)
print(resp.text)
