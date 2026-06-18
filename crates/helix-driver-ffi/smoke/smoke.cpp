// helix C++ 直连冒烟（cpp → rust → cpp）。
// 真 C++ 程序 link helix cdylib + #include helix.hpp：建 handle(传 vtable) → 下命令 →
// 收 on_domain_events_batch 批回调 → 断言含 im:post:sending（命令→step→Emit→批回 C++ 端到端）。
// 离线可跑：im_send_message 触发的是本地 optimistic emit，不依赖真服务器。
#include "helix.hpp"
#include <cstdint>
#include <cstring>
#include <cstdio>
#include <string>
#include <vector>
#include <mutex>
#include <thread>
#include <chrono>

static std::mutex g_mu;
static std::vector<std::string> g_batches; // 收到的批 buffer（原样拷贝）

// 批回调：HX-C007 整批一次过边界。立即拷贝（ptr 仅本次调用有效）。
extern "C" void on_batch(void* /*ctx*/, const uint8_t* ptr, uintptr_t len) {
    std::lock_guard<std::mutex> lk(g_mu);
    g_batches.emplace_back(reinterpret_cast<const char*>(ptr), len);
}
// 注入时间（确定性，HX-C002）。
extern "C" uint64_t fixed_now_ms(void* /*ctx*/) { return 1700000000000ULL; }

int main() {
    printf("[helix C++ smoke] cpp -> rust -> cpp 直连冒烟\n");

    CppHostVtable vtable;
    vtable.on_domain_events_batch = on_batch;
    vtable.now_ms = fixed_now_ms;
    vtable.ctx = nullptr;

    const char* api = "https://api.example.test";
    const char* ws  = "wss://ws.example.test/socket";
    HelixHandle* h = helix_create_with_network(
        reinterpret_cast<const uint8_t*>(api), std::strlen(api),
        reinterpret_cast<const uint8_t*>(ws),  std::strlen(ws),
        vtable);
    if (!h) { fprintf(stderr, "FAIL: helix_create_with_network 返回 null\n"); return 1; }
    printf("  ✓ create handle（storage 开库 + pump 起线程）\n");

    const char* name = "im_send_message";
    const char* payload =
        "{\"channel_id\":\"01HQ0000000000000000000000\",\"temporary_id\":\"tmp-1\",\"text\":\"hi\"}";
    int32_t st = helix_command(
        h, reinterpret_cast<const uint8_t*>(name), std::strlen(name),
        reinterpret_cast<const uint8_t*>(payload), std::strlen(payload));
    if (st != FFI_OK) { fprintf(stderr, "FAIL: helix_command status=%d\n", st); helix_destroy(h); return 1; }
    printf("  ✓ helix_command(im_send_message) 入队 OK\n");

    // 轮询等 pump（独立线程）step + flush 批回调到达。
    bool got = false; size_t seen = 0;
    for (int i = 0; i < 200 && !got; ++i) {
        {
            std::lock_guard<std::mutex> lk(g_mu);
            seen = g_batches.size();
            for (auto& b : g_batches)
                if (b.find("im:post:sending") != std::string::npos) { got = true; break; }
        }
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }

    helix_destroy(h); // graceful drain + join pump 线程（回调不再访问 ctx）
    printf("  ✓ helix_destroy（join pump 线程，干净销毁）\n");

    if (got) {
        printf("[helix C++ smoke] PASS ✅ — 真 C++ → cdylib → im:post:sending 批回调（cpp→rust→cpp 端到端通）\n");
        return 0;
    }
    fprintf(stderr, "[helix C++ smoke] FAIL ❌ — %zu 批回调中无 im:post:sending\n", seen);
    return 1;
}
