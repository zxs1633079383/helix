import { lookup } from 'node:dns/promises';
import net from 'node:net';
import { mkdir, appendFile } from 'node:fs/promises';
import { performance } from 'node:perf_hooks';
import { resolveMessagev3Environment, MESSAGEV3_PRE_CREDENTIAL_ACCOUNTS } from './real-chain-environment.mjs';

const env = resolveMessagev3Environment('pre');
const directory = 'artifacts/messagev3-diagnose';
await mkdir(directory, { recursive: true });
let failures = 0;

/** 将限时探针结果逐条落盘，仅保存状态与错误码，不保存认证响应。 */
async function probe(stage, task, milliseconds = 8000) {
    const start = performance.now();
    console.log(`[DIAG] START ${stage}`);
    let timer;
    try {
        const value = await Promise.race([task(), new Promise((_, reject) => {
            timer = setTimeout(() => reject(Object.assign(new Error('deadline'), { code: 'DEADLINE' })), milliseconds);
        })]);
        const record = { stage, ok: true, elapsed_ms: Math.round(performance.now() - start), ...value };
        console.log(JSON.stringify(record));
        await appendFile(`${directory}/probes.jsonl`, JSON.stringify(record) + '\n');
    } catch (error) {
        failures++;
        const record = { stage, ok: false, elapsed_ms: Math.round(performance.now() - start), code: error.cause?.code ?? error.code ?? error.name };
        console.log(JSON.stringify(record));
        await appendFile(`${directory}/probes.jsonl`, JSON.stringify(record) + '\n');
    } finally { clearTimeout(timer); }
}

/** TCP 探针自行释放 socket，保证失败连接不会拖住 Node 退出。 */
function tcp(url) {
    return new Promise((resolve, reject) => {
        const socket = net.connect({ host: url.hostname, port: Number(url.port || (url.protocol === 'https:' || url.protocol === 'wss:' ? 443 : 80)) });
        socket.setTimeout(5000);
        socket.once('connect', () => { socket.destroy(); resolve({ connected: true }); });
        socket.once('error', error => { socket.destroy(); reject(error); });
        socket.once('timeout', () => { socket.destroy(); reject(Object.assign(new Error('timeout'), { code: 'TCP_TIMEOUT' })); });
    });
}

for (const [name, endpoint] of [['Go', env.goHttpBase], ['Java', env.javaApiBase], ['WS', env.goWsUrl]]) {
    const url = new URL(endpoint);
    await probe(`${name} DNS ${url.hostname}`, async () => ({ addresses: await lookup(url.hostname, { all: true }) }));
    await probe(`${name} TCP ${url.hostname}`, () => tcp(url));
}
await probe('Go health', async () => {
    const response = await fetch(env.goHealthUrl, { signal: AbortSignal.timeout(7000) });
    const body = await response.json();
    if (!response.ok || body.status !== 'OK') throw Object.assign(new Error('health'), { code: `HTTP_${response.status}` });
    return { http_status: response.status, business_status: body.status };
});
// 与 Rust 测试相同的账号及请求头；只验证第一个账号，避免静默遍历导致长等待。
await probe('Java login first-account', async () => {
    const account = MESSAGEV3_PRE_CREDENTIAL_ACCOUNTS[0];
    const response = await fetch(`${env.javaApiBase}/User/login`, {
        method: 'POST', signal: AbortSignal.timeout(7000),
        headers: { 'Content-Type': 'application/json', appType: 'bct', device: 'IOS', language: 'zh', appVersion: '1.0.120', timeZone: 'Asia/Shanghai', osVersion: '16.5.1', siteName: 'b', envType: 'pre', deviceId: 'helix-phase1-rust' },
        body: JSON.stringify({ userName: account.mobile, password: account.password })
    });
    const body = await response.json();
    const user = body.data ?? body;
    const userId = user.userId ?? user.user_id ?? user.id;
    const cookieId = user.cookieId ?? user.cookieid ?? user.cookie_id ?? userId;
    if (!response.ok || !userId || String(cookieId) !== String(userId)) throw Object.assign(new Error('login'), { code: `HTTP_${response.status}_INVALID_IDENTITY` });
    return { http_status: response.status, identity_valid: true };
});
console.log(`[DIAG] FINISH failures=${failures}; G07B=${failures ? 'BLOCKED_PRECONDITION' : 'NEXT'}`);
// DNS 原生解析线程可能仍等待系统超时，完成证据写入后显式结束诊断进程。
process.exit(failures ? 1 : 0);
