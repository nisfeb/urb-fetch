//  wasi.js: the WASI imports mesa.wasm and render.wasm ask for, each one
//  given a real answer (docs/rust-core.md, M6's findings, lists who asks
//  for which). Nothing here is a stub that claims success it didn't
//  have: there are no files, so a file call is answered as WASI answers
//  for a descriptor that isn't open.
//
//    random_get            the standard library's hash seeds (HashMap's
//                          RandomState, serde_json's maps): real random
//                          bytes. They never reach an output; the
//                          engine's entropy is the host's (host.entropy)
//    environ_sizes_get,    the environment: env's entries, so MESA_TRACE=1
//    environ_get           turns libmesa's packet log on, as before
//    fd_write              stdout and stderr, as lines to log (a panic's
//                          message, libmesa's trace); EBADF for any other
//    fd_close, fd_seek     stdio: closing it succeeds and seeking it is
//                          ESPIPE; EBADF for any other descriptor
//    fd_prestat_get,       wasi-libc's scan for preopened directories:
//    fd_prestat_dir_name   EBADF, there are none
//    proc_exit             an exit (abort included) is thrown as an error

const ESUCCESS = 0, EBADF = 8, ESPIPE = 70
const STDIO = (fd) => fd === 0 || fd === 1 || fd === 2

export function wasi(memory, log = console.error, env = {}, name = 'mesa.wasm') {
  const enc = new TextEncoder(), dec = new TextDecoder()
  const pending = { 1: '', 2: '' }
  const vars = Object.entries(env).map(([k, v]) => enc.encode(`${k}=${v}\0`))
  const view = () => new DataView(memory().buffer)
  return {
    random_get(buf, len) {
      const m = new Uint8Array(memory().buffer, buf, len)
      for (let at = 0; at < len; at += 65536) crypto.getRandomValues(m.subarray(at, Math.min(len, at + 65536)))
      return ESUCCESS
    },
    environ_sizes_get(count, size) {
      const v = view()
      v.setUint32(count, vars.length, true)
      v.setUint32(size, vars.reduce((n, b) => n + b.length, 0), true)
      return ESUCCESS
    },
    environ_get(ptrs, buf) {
      const v = view()
      vars.forEach((b, i) => {
        v.setUint32(ptrs + 4 * i, buf, true)
        new Uint8Array(memory().buffer).set(b, buf)
        buf += b.length
      })
      return ESUCCESS
    },
    fd_write(fd, iovs, n, written) {
      if (fd !== 1 && fd !== 2) return EBADF
      const v = view()
      let total = 0
      for (let i = 0; i < n; i++) {
        const p = v.getUint32(iovs + 8 * i, true), len = v.getUint32(iovs + 8 * i + 4, true)
        pending[fd] += dec.decode(new Uint8Array(memory().buffer, p, len))
        total += len
      }
      const lines = pending[fd].split('\n')
      pending[fd] = lines.pop()
      for (const l of lines) log(l.replace(/\r$/, ''))
      view().setUint32(written, total, true)
      return ESUCCESS
    },
    fd_close: (fd) => (STDIO(fd) ? ESUCCESS : EBADF),
    fd_seek: (fd) => (STDIO(fd) ? ESPIPE : EBADF),
    fd_prestat_get: () => EBADF,
    fd_prestat_dir_name: () => EBADF,
    proc_exit(code) {
      throw new Error(`${name} exited with ${code}`)
    },
  }
}

//  The WASI imports a module asks for that wasi() does not answer: none
//  may be left, or instantiating would fail on a missing import, or
//  worse, a host would have to stub it.
export function unanswered(module) {
  const have = new Set(Object.keys(wasi(() => null)))
  return WebAssembly.Module.imports(module)
    .filter((i) => i.module === 'wasi_snapshot_preview1' && !have.has(i.name))
    .map((i) => i.name)
}
