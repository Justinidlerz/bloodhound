const binding = require('#binding');
const { fetch, ProxyAgent } = require('undici');

function reason(error) {
  return error?.cause?.code || error?.code || error?.name || 'http-request-failed';
}

// Only transport runs in Node. Rust still owns access checks, redirect policy,
// retries, dependency discovery and scan budgets.
async function requestHttp(serialized, dispatcher, signal) {
  const request = JSON.parse(serialized);
  const timeout = AbortSignal.timeout(request.timeout);
  const headers = new Headers({ 'user-agent': request.userAgent });
  for (const [name, value] of Object.entries(request.headers || {})) headers.set(name, value);
  let response;
  try {
    response = await fetch(request.url, {
      redirect: 'manual',
      headers,
      dispatcher,
      signal: signal ? AbortSignal.any([signal, timeout]) : timeout,
    });
  } catch (error) {
    throw new Error(reason(error));
  }
  const result = {
    status: response.status,
    location: response.headers.get('location'),
    content: { Ok: '' },
  };
  try {
    if (response.ok) {
      const length = response.headers.get('content-length');
      if (
        !response.headers.has('content-encoding') &&
        length &&
        Number(length) > request.maxBytes
      ) {
        result.content = { Err: 'resource-byte-budget' };
      } else {
        const chunks = [];
        let bytes = 0;
        for await (const chunk of response.body || []) {
          bytes += chunk.byteLength;
          if (bytes > request.maxBytes) {
            result.content = { Err: 'resource-byte-budget' };
            break;
          }
          chunks.push(chunk);
        }
        if (!result.content.Err) {
          try {
            result.content = {
              Ok: new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(
                Buffer.concat(chunks, bytes),
              ),
            };
          } catch {
            result.content = { Err: 'invalid-utf8' };
          }
        }
      }
    }
  } catch (error) {
    result.content = { Err: reason(error) };
  } finally {
    if (response.body && !response.body.locked) await response.body.cancel().catch(() => {});
  }
  return JSON.stringify(result);
}

async function scan(method, input, options = {}) {
  const dispatcher = options.proxy ? new ProxyAgent(options.proxy) : undefined;
  try {
    return await binding[method](input, {
      ...options,
      __bloodhoundHttp: (request) => requestHttp(request, dispatcher, options.signal),
    });
  } finally {
    await dispatcher?.close();
  }
}

// Preserve generated lifecycle symbols as well as the public binding exports.
const descriptors = Object.getOwnPropertyDescriptors(binding);
for (const method of ['scanRaw', 'scanHtmlRaw', 'scanJsRaw']) {
  descriptors[method] = {
    value: (input, options) => scan(method, input, options),
    enumerable: true,
  };
}
module.exports = Object.defineProperties({}, descriptors);
