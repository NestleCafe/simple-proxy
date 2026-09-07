'use strict';

const http = require('http');
const https = require('https');
const fs = require('fs');
const path = require('path');

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
 * 读取并解析 JSON 配置文件。
 * @param {string} filePath 配置文件绝对路径
 * @returns {object} 解析后的配置对象
 */
function loadJsonConfig(filePath) {
  try {
    return JSON.parse(fs.readFileSync(filePath, 'utf8'));
  } catch (err) {
    console.error(`读取配置文件失败: ${filePath}`);
    throw err;
  }
}

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
 * 创建反向代理请求处理函数。
 * @param {URL} targetUrl 目标地址
 * @param {object} staticHeaders 需要附加的静态请求头
 * @param {import('http').Agent} agent 复用连接用的 Agent
 * @returns {Function} HTTP 请求处理回调
 */
function createProxyHandler(targetUrl, staticHeaders, agent) {
  return (clientReq, clientRes) => {
    const startTime = Date.now();
    const forwardHeaders = buildForwardHeaders(
      clientReq.headers,
      staticHeaders,
      targetUrl.host
    );

    // 保留客户端原始路径与查询参数
    const forwardPath = clientReq.url;

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

    // 将客户端请求体流式转发给上游
    clientReq.pipe(upstreamReq);
  };
}

/**
 * 启动反向代理服务。
 * @param {object} config 代理配置（target/httpPort）
 * @param {object} staticHeaders 附加的静态请求头
 */
function startProxy(config, staticHeaders) {
  const targetUrl = new URL(config.target);
  const agent = createAgent(targetUrl);
  const handler = createProxyHandler(targetUrl, staticHeaders, agent);

  const httpServer = http.createServer(handler);
  httpServer.on('error', (err) => {
    log(`服务启动失败: ${err.message}`);
    process.exit(1);
  });
  httpServer.listen(config.httpPort, () => {
    log(`反向代理已启动: http://0.0.0.0:${config.httpPort} => ${config.target}`);
    log('请将 AI Agent 的接口地址指向该地址，代理会自动附加配置的请求头并转发目标地址');
  });
}

/**
 * 程序入口：加载配置并启动代理。
 */
function main() {
  const configFile = path.resolve(__dirname, 'proxy.config.json');

  try {
    const config = loadJsonConfig(configFile);

    if (!config.target) {
      throw new Error('proxy.config.json 中缺少 "target" 配置项');
    }

    startProxy(config, config.headers || {});
  } catch (err) {
    console.error(`启动失败: ${err.message}`);
    process.exit(1);
  }
}

main();