// 真 C++ → cdylib → C++ batch callback；Node fixture 校验 wire/JSON 与真实 HTTP。
#include "helix.hpp"
#include <cerrno>
#include <chrono>
#include <cstdio>
#include <cstring>
#include <mutex>
#include <poll.h>
#include <string>
#include <thread>
#include <unistd.h>
#include <utility>
#include <vector>

constexpr size_t MAX_BATCH_BYTES = 1024 * 1024;
struct BatchCapture {
    std::mutex mutex;
    std::vector<std::string> pending;
    bool invalid = false;
};

extern "C" void on_batch(void* ctx, const uint8_t* ptr, uintptr_t len) {
    auto& capture = *static_cast<BatchCapture*>(ctx);
    std::lock_guard<std::mutex> lock(capture.mutex);
    if (!ptr || len < 4 || len > MAX_BATCH_BYTES || capture.pending.size() >= 64) {
        capture.invalid = true;
        return;
    }
    // 借用指针只在本回调有效，先原样拷贝；不在 pump 回调线程做 IO。
    capture.pending.emplace_back(reinterpret_cast<const char*>(ptr), len);
}

extern "C" uint64_t fixed_now_ms(void*) { return 1700000000000ULL; }

struct HandleGuard {
    HelixHandle* handle;
    explicit HandleGuard(HelixHandle* value) : handle(value) {}
    HandleGuard(const HandleGuard&) = delete;
    HandleGuard& operator=(const HandleGuard&) = delete;
    ~HandleGuard() {
        helix_destroy(handle); // join pump 后，main 的 callback ctx 仍然存活。
        std::puts("DESTROYED");
        std::fflush(stdout);
    }
};

bool forward_batches(BatchCapture& capture) {
    std::vector<std::string> batches;
    {
        std::lock_guard<std::mutex> lock(capture.mutex);
        if (capture.invalid) return false;
        batches.swap(capture.pending);
    }
    constexpr char HEX[] = "0123456789abcdef";
    for (const auto& batch : batches) {
        std::string line = "BATCH ";
        line.reserve(7 + batch.size() * 2);
        for (unsigned char byte : batch) {
            line.push_back(HEX[byte >> 4]);
            line.push_back(HEX[byte & 15]);
        }
        line.push_back('\n');
        if (std::fwrite(line.data(), 1, line.size(), stdout) != line.size()) return false;
    }
    return std::fflush(stdout) == 0;
}

// Node 的 SEND/SENDING/DONE 握手；非阻塞读取，EOF/错误走正常 RAII 退出。
struct ControlInput {
    std::string pending;
    int read_line(std::string& line) {
        auto take_line = [&]() {
            const auto end = pending.find('\n');
            if (end == std::string::npos) return false;
            line = pending.substr(0, end);
            pending.erase(0, end + 1);
            return true;
        };
        if (take_line()) return 1;
        pollfd input{STDIN_FILENO, POLLIN, 0};
        const int ready = ::poll(&input, 1, 0);
        if (ready < 0) return errno == EINTR ? 0 : -1;
        if (ready == 0) return 0;
        char bytes[64];
        const auto count = ::read(STDIN_FILENO, bytes, sizeof(bytes));
        if (count < 0 && errno == EINTR) return 0;
        if (count <= 0 || pending.size() + static_cast<size_t>(count) > 64) return -1;
        pending.append(bytes, static_cast<size_t>(count));
        return take_line() ? 1 : 0;
    }
};

bool wait_for_control(BatchCapture& capture, ControlInput& input, const char* expected) {
    // 保留原 sending callback 的 200×10ms 上界；hydration/HTTP 各有独立同等上界。
    for (int i = 0; i < 200; ++i) {
        if (!forward_batches(capture)) return false;
        std::string line;
        const int status = input.read_line(line);
        if (status < 0) return false;
        if (status == 1) return line == expected;
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    std::fprintf(stderr, "FAIL: timeout waiting for typed fixture %s\n", expected);
    return false;
}

bool require_ok(int32_t status, const char* operation) {
    if (status == FFI_OK) return true;
    std::fprintf(stderr, "FAIL: %s status=%d\n", operation, status);
    return false;
}

bool command(HelixHandle* handle, const char* name, const char* payload) {
    return require_ok(helix_command(
        handle, reinterpret_cast<const uint8_t*>(name), std::strlen(name),
        reinterpret_cast<const uint8_t*>(payload), std::strlen(payload)), name);
}

bool run(BatchCapture& capture, const char* go_api) {
    CppHostVtable vtable{on_batch, fixed_now_ms, &capture};
    const char* java_api = "https://java.example.test";
    const char* ws = "wss://go.example.test/api/v4/websocket";
    HelixHandle* handle = helix_create_with_endpoints(
        reinterpret_cast<const uint8_t*>(go_api), std::strlen(go_api),
        reinterpret_cast<const uint8_t*>(java_api), std::strlen(java_api),
        reinterpret_cast<const uint8_t*>(ws), std::strlen(ws), vtable);
    if (!handle) {
        std::fprintf(stderr, "FAIL: helix_create_with_endpoints returned null\n");
        return false;
    }
    HandleGuard guard(handle);
    for (const auto& header : {std::pair{"cookieId", "viewer-1"}, std::pair{"companyId", "team-1"}}) {
        if (!require_ok(helix_set_header(
            handle, reinterpret_cast<const uint8_t*>(header.first), std::strlen(header.first),
            reinterpret_cast<const uint8_t*>(header.second), std::strlen(header.second)), "set_header")) return false;
    }
    if (!command(handle, "im_ensure_channel_loaded",
        "{\"channel_id\":\"01hq0000000000000000000000\",\"req_id\":\"seed-owned-channel\"}")) return false;
    std::puts("HYDRATING");
    ControlInput input;
    if (!wait_for_control(capture, input, "SEND")) return false;

    // 原 send payload 不变；Node 只在真实持久化读回后放行此公开命令。
    if (!command(handle, "im_send_message",
        "{\"channel_id\":\"01hq0000000000000000000000\",\"temporary_id\":\"tmp-1\",\"text\":\"hi\"}")) return false;
    std::puts("SENT");
    if (!wait_for_control(capture, input, "SENDING")) return false;
    return wait_for_control(capture, input, "DONE");
}

int main(int argc, char** argv) {
    if (argc != 2) {
        std::fprintf(stderr, "usage: helix_smoke <fixture-go-api>\n");
        return 1;
    }
    BatchCapture capture; // ctx 比 run() 中的 HandleGuard 更长寿，包括全部失败分支。
    const bool passed = run(capture, argv[1]);
    const bool forwarded = forward_batches(capture); // destroy 的最后一批仍交 typed oracle。
    return passed && forwarded ? 0 : 1;
}
