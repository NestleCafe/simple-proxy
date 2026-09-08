'use strict';

import http from 'http';
import https from 'https';
import proxyConfigs from './proxy.config.js';

// hop-by-hop 请求头属于逐跳传输信息，不应透传给上游服务器
const HOP_BY_HOP_HEADERS = [
  'connection',
  'keep-alive',
  'proxy-authenticate',
  'proxy-authorization',
  'te',
  'trailer',
  'transfer-encoding',
  'upgrade',
];

/**
 * 输出带时间戳的结构化日志。
 * @param {string} message 日志内容
 */
function log(message) {
  console.log(`[${new Date().toISOString()}] ${message}`);
}

/**
 * 根据目标 URL 的协议创建对应的 keep-alive 请求 Agent，用于复用连接。
 * @param {URL} targetUrl 解析后的目标 URL
 * @returns {import('http').Agent} http 或 https 的 Agent 实例
 */
function createAgent(targetUrl) {
  return targetUrl.protocol === 'https:'
    ? new https.Agent({ keepAlive: true })
    : new http.Agent({ keepAlive: true });
}

/**
 * 组装转发给上游服务器的请求头：合并透传的原请求头与配置的静态附加头。
 * @param {object} clientHeaders 客户端原始请求头
 * @param {object} staticHeaders 配置文件中需要附加的静态请求头
 * @param {string} targetHost 目标服务器的 host（替换原 host）
 * @returns {object} 组装后的转发请求头
 */
function buildForwardHeaders(clientHeaders, staticHeaders, targetHost) {
  const headers = {};
  const seen = new Set();

  // 1. 透传客户端原始请求头（排除 hop-by-hop 头与 host）
  for (const [name, value] of Object.entries(clientHeaders)) {
    const lower = name.toLowerCase();
    if (lower === 'host' || HOP_BY_HOP_HEADERS.includes(lower)) continue;
    headers[name] = value;
    seen.add(lower);
  }

  // 2. 附加配置的静态请求头；与透传头同名时以静态配置为准
  for (const [name, value] of Object.entries(staticHeaders)) {
    headers[name] = value;
    seen.add(name.toLowerCase());
  }

  // 3. 使用目标 host 覆盖原 host
  headers.Host = targetHost;

  // 4. 移除会被 Node 自动计算而可能造成冲突的 content-length（body 由 pipe 流式转发时 Node 会重新处理）
  if (seen.has('content-length')) delete headers['content-length'];

  return headers;
}

/**
 * 根据协议在请求体中注入推理强度默认值（仅当客户端未显式指定时）。
 * @param {object|null} body 解析后的 JSON 请求体
 * @param {string} protocol 目标协议：openai / anthropic
 * @param {string} reasoningEffort 配置的推理强度，如 max
 * @returns {boolean} 是否发生了注入
 */
function injectReasoningEffort(body, protocol, reasoningEffort) {
  if (!body || typeof body !== 'object') return false;

  if (protocol === 'openai') {
    // OpenAI 协议
    // reasoning_effort 推理强度
    if (body.reasoning_effort === undefined) {
      body.reasoning_effort = reasoningEffort;
      return true;
    }
  } else if (protocol === 'anthropic') {
    // Anthropic 协议
    if (body.output_config == undefined) {
      // thinking.type: enabled（开启思考模式）/ disabled（关闭思考模式）
      if (body.thinking == undefined) {
        body.thinking = { type: 'enabled' };
      }

      body.output_config = { effort: reasoningEffort };
      return true;
    }
  }

  return false;
}

/**
 * 创建反向代理请求处理函数。
 * @param {URL} targetUrl 目标地址
 * @param {object} staticHeaders 需要附加的静态请求头
 * @param {import('http').Agent} agent 复用连接用的 Agent
 * @param {object} [options] 可选配置
 * @param {string} [options.protocol] 目标协议（openai / anthropic），默认 openai
 * @param {string} [options.reasoningEffort] 推理强度默认值；配置后会在请求体中注入
 * @returns {Function} HTTP 请求处理回调
 */
function createProxyHandler(targetUrl, staticHeaders, agent, options = {}) {
  const { protocol = 'openai', reasoningEffort } = options;
  // 仅当配置了推理强度且协议受支持时才需要改写请求体
  const shouldInject =
    reasoningEffort && (protocol === 'openai' || protocol === 'anthropic');
  return (clientReq, clientRes) => {
    const startTime = Date.now();
    const forwardHeaders = buildForwardHeaders(
      clientReq.headers,
      staticHeaders,
      targetUrl.host
    );

    // 将客户端原始路径与查询参数拼接到 target 的路径之后。
    // 例如 target 为 https://opencode.ai/zen/go/v1 时，
    // 请求 /chat/completions 会转发到 https://opencode.ai/zen/go/v1/chat/completions
    const targetBasePath =
      targetUrl.pathname === '/' ? '' : targetUrl.pathname.replace(/\/+$/, '');
    const forwardPath = targetBasePath + clientReq.url;

    const options = {
      protocol: targetUrl.protocol,
      hostname: targetUrl.hostname,
      port: targetUrl.port || (targetUrl.protocol === 'https:' ? 443 : 80),
      method: clientReq.method,
      path: forwardPath,
      headers: forwardHeaders,
      agent,
    };

    const upstreamReq = (targetUrl.protocol === 'https:' ? https : http).request(options, (upstreamRes) => {
      clientRes.writeHead(upstreamRes.statusCode, upstreamRes.headers);
      upstreamRes.pipe(clientRes);

      // 响应完成时输出访问日志
      upstreamRes.on('end', () => {
        const duration = Date.now() - startTime;
        log(
          `[${clientReq.method}] ${clientReq.url} => ${targetUrl.origin}${forwardPath} ` +
            `状态码 ${upstreamRes.statusCode} 耗时 ${duration}ms`
        );
      });
    });

    // 客户端断开连接时，中止上游请求，避免资源泄漏
    clientReq.on('aborted', () => upstreamReq.destroy());

    // 上游请求出错（超时、连接被拒等）时返回 502
    upstreamReq.on('error', (err) => {
      log(`[${clientReq.method}] ${clientReq.url} 转发失败: ${err.message}`);
      if (!clientRes.headersSent) {
        clientRes.writeHead(502, { 'Content-Type': 'text/plain; charset=utf-8' });
        clientRes.end('Bad Gateway: ' + err.message);
      } else {
        clientRes.destroy();
      }
    });

    // 无需改写请求体时，直接流式转发，保持最小延迟与内存占用
    if (!shouldInject) {
      clientReq.pipe(upstreamReq);
      return;
    }

    // 需要注入推理强度：缓冲请求体，解析 JSON 并注入默认值后转发
    const chunks = [];
    clientReq.on('data', (chunk) => chunks.push(chunk));
    clientReq.on('end', () => {
      const body = Buffer.concat(chunks);
      let finalBody = body;

      if (body.length) {
        try {
          const parsed = JSON.parse(body.toString('utf8'));
          if (injectReasoningEffort(parsed, protocol, reasoningEffort)) {
            finalBody = Buffer.from(JSON.stringify(parsed), 'utf8');
            // 注入成功无需打印，避免刷屏；仅注入失败（JSON 解析失败）时记录日志
            // log(`已注入推理强度: ${protocol} reasoning_effort=${reasoningEffort}`);
          }
        } catch (err) {
          log(`请求体 JSON 解析失败，跳过推理强度注入: ${err.message}`);
        }
      }

      // 缓冲模式下显式设置 content-length，避免使用 chunked 编码
      upstreamReq.setHeader('Content-Length', finalBody.length);
      upstreamReq.end(finalBody);
    });
  };
}

/**
 * 启动反向代理服务。
 * @param {object} config 代理配置（target/httpPort/protocol/reasoningEffort）
 * @param {object} staticHeaders 附加的静态请求头
 */
function startProxy(config, staticHeaders) {
  const targetUrl = new URL(config.target);
  const agent = createAgent(targetUrl);
  const handler = createProxyHandler(targetUrl, staticHeaders, agent, {
    protocol: config.protocol || 'openai',
    reasoningEffort: config.reasoningEffort,
  });

  const httpServer = http.createServer(handler);
  httpServer.on('error', (err) => {
    log(`服务启动失败: ${err.message}`);
    process.exit(1);
  });
  httpServer.listen(config.httpPort, () => {
    log(`反向代理已启动: http://127.0.0.1:${config.httpPort} => ${config.target}`);
  });
}

/**
 * 程序入口：加载配置并启动一个或多个代理实例（多端口并行）。
 */
function main() {
  try {
    // 兼容单个对象与数组两种写法，统一视为代理实例列表
    const proxies = Array.isArray(proxyConfigs) ? proxyConfigs : [proxyConfigs];

    if (!proxies.length) {
      throw new Error('proxy.config.js 中未配置任何代理实例');
    }

    proxies.forEach((proxyConfig, index) => {
      if (!proxyConfig.target) {
        throw new Error(`proxy.config.js 第 ${index + 1} 个配置缺少 "target" 配置项`);
      }
      startProxy(proxyConfig, proxyConfig.headers || {});
    });
  } catch (err) {
    console.error(`启动失败: ${err.message}`);
    process.exit(1);
  }
}

main();