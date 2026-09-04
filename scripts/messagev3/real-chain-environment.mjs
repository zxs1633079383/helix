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

// PRE 测试账号由 real-chain 自持有；仅用于 CI/本地验收，不进入业务产物。
export const MESSAGEV3_PRE_CREDENTIAL_ACCOUNTS = Object.freeze([
    { mobile: '678', password: '678' },
    { mobile: '666', password: '666' },
    { mobile: '15899121571', password: 'startup.2022' },
    { mobile: '445', password: '445' },
    { mobile: '123456', password: '123456' },
    { mobile: '999', password: '999' },
    { mobile: '789', password: '789' },
    { mobile: '555', password: '555' },
    { mobile: '444', password: '444' },
    { mobile: '123', password: '123' },
    { mobile: '447', password: '447' },
    { mobile: '446', password: '446' },
    { mobile: '456', password: '456' },
    { mobile: '333', password: '333' },
    { mobile: '777', password: '777' },
    { mobile: '111', password: '111' }
]);

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

/** 为 PRE 注入内置账号池，同时允许显式 JSON/File 覆盖默认值。 */
export function resolveMessagev3CredentialEnvironment(profileName, environment = process.env) {
    const inlineCredentials = environment.CSES_WDIO_CREDENTIALS_JSON?.trim();
    if (inlineCredentials) {
        return Object.freeze({ CSES_WDIO_CREDENTIALS_JSON: inlineCredentials });
    }
    if (environment.CSES_WDIO_CREDENTIALS_FILE?.trim() || profileName !== 'pre') {
        return Object.freeze({});
    }
    return Object.freeze({
        CSES_WDIO_CREDENTIALS_JSON: JSON.stringify(MESSAGEV3_PRE_CREDENTIAL_ACCOUNTS)
    });
}
