/** MessageV3 real-chain 的可配置运行环境；新增环境只需扩展此表。 */
export const MESSAGEV3_ENVIRONMENTS = Object.freeze({
    pre: Object.freeze({
        goHttpBase: 'http://api-im-pre.jinqidongli.com/api/cses',
        goWsUrl: 'ws://ws-im-pre.jinqidongli.com/api/v4/websocket',
        javaApiBase: 'https://api-pre.jinqidongli.com'
    }),
    local: Object.freeze({
        goHttpBase: 'http://127.0.0.1:18066/api/cses',
        goWsUrl: 'ws://127.0.0.1:18066/api/v4/websocket',
        javaApiBase: 'http://127.0.0.1:3399'
    })
});

export const DEFAULT_MESSAGEV3_ENVIRONMENT = 'pre';

/** 规范化环境名，避免空值或大小写差异绕过 profile 选择。 */
function normalizeEnvironmentName(value) {
    return String(value ?? DEFAULT_MESSAGEV3_ENVIRONMENT).trim().toLowerCase() || DEFAULT_MESSAGEV3_ENVIRONMENT;
}

/** 解析 profile，并允许 CI/本地用同名变量覆盖单个端点。 */
export function resolveMessagev3Environment(value = process.env.MESSAGEV3_ENV) {
    const name = normalizeEnvironmentName(value);
    const configuredGoHttpBase = process.env.MESSAGEV3_GO_HTTP_BASE?.trim();
    const configuredGoWsUrl = process.env.MESSAGEV3_GO_WS_URL?.trim();
    const configuredJavaApiBase = process.env.MESSAGEV3_JAVA_API_BASE?.trim()
        || process.env.HELIX_MAIN_API_BASE?.trim();
    const profile = MESSAGEV3_ENVIRONMENTS[name] || (
        configuredGoHttpBase && configuredGoWsUrl && configuredJavaApiBase
            ? { goHttpBase: configuredGoHttpBase, goWsUrl: configuredGoWsUrl, javaApiBase: configuredJavaApiBase }
            : undefined
    );
    if (!profile) {
        throw new Error(`不支持的 MessageV3 环境: ${name}；可选值: ${Object.keys(MESSAGEV3_ENVIRONMENTS).join(', ')}，或同时提供 MESSAGEV3_GO_HTTP_BASE/MESSAGEV3_GO_WS_URL/MESSAGEV3_JAVA_API_BASE`);
    }
    const goHttpBase = configuredGoHttpBase || profile.goHttpBase;
    const goWsUrl = configuredGoWsUrl || profile.goWsUrl;
    const javaApiBase = configuredJavaApiBase || profile.javaApiBase;
    return Object.freeze({
        name,
        goHttpBase,
        goWsUrl,
        goHealthUrl: process.env.MESSAGEV3_GO_HEALTH_URL?.trim() || `${goHttpBase}/health`,
        javaApiBase
    });
}
