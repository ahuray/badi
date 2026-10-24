import { timingSafeEqual } from 'node:crypto';

/** Every Lab response: same-origin page resources only, never framed or cached. */
export const CONTENT_SECURITY_POLICY = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

export function setSecurityHeaders(response) {
  response.setHeader('Cache-Control', 'no-store');
  response.setHeader('X-Content-Type-Options', 'nosniff');
  response.setHeader('Cross-Origin-Resource-Policy', 'same-origin');
  response.setHeader('Referrer-Policy', 'no-referrer');
  response.setHeader('Content-Security-Policy', CONTENT_SECURITY_POLICY);
}

/** Whether the request names the Lab's own local host and, when it sends one, origin. */
export function fromLabOrigin(request, origin) {
  return request.headers.host === new URL(origin).host && (!request.headers.origin || request.headers.origin === origin);
}

/** Whether the request carries the served page's capability token, compared in constant time. */
export function hasLabToken(request, capability) {
  const token = Buffer.from(String(request.headers['x-badi-lab-token'] ?? ''));
  return token.length === capability.length && timingSafeEqual(token, Buffer.from(capability));
}

/** An API request from the served page: its origin and its token. */
export const fromLabPage = (request, origin, capability) => fromLabOrigin(request, origin) && hasLabToken(request, capability);

/** A JSON reply, unless the client already went away or a reply was sent. */
export function sendJSON(response, status, value) {
  if (response.destroyed || response.writableEnded) return;
  response.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8' });
  response.end(JSON.stringify(value));
}

/**
 * The request body as JSON: at most `limit` bytes of strict UTF-8. Reading
 * stops at the limit. `tooLarge` and `malformed` build each route's own error.
 */
export async function readJSON(request, limit, { tooLarge, malformed }) {
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > limit) throw tooLarge();
    chunks.push(chunk);
  }
  try { return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(chunks))); }
  catch { throw malformed(); }
}
