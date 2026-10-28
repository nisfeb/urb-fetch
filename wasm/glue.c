//  glue.c: libmesa's engine as a wasm module for a JavaScript host.
//
//  The engine's structs (ships, lanes, points, identities) stay in C. A
//  ship crosses to JavaScript as two 64-bit halves, a noun as its 64-bit
//  handle, a string as a pointer to NUL-terminated UTF-8. The hooks call
//  the host's imports, and the host calls the exports below. One engine
//  per module, run by a comet. Its store is libmesa's in-memory one, and
//  the host saves what each commit changed.

#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "mesa/engine.h"
#include "mesa/gall.h"

#define EXPORT(n) __attribute__((export_name(n)))
#define IMPORT(n) __attribute__((import_module("host"), import_name(n)))

IMPORT("send") void host_send(int kind, int galaxy, uint32_t ip, int port,
                              const uint8_t* buf, size_t len);
IMPORT("need_point") void host_need_point(uint64_t lo, uint64_t hi);
IMPORT("on_poke") int host_on_poke(uint64_t lo, uint64_t hi, uint64_t bone,
                                   const char* agent, mesa_noun mark,
                                   mesa_noun noun);
IMPORT("on_done") void host_on_done(uint64_t lo, uint64_t hi, uint64_t bone,
                                    mesa_noun error);
IMPORT("on_sage") void host_on_sage(uint64_t lo, uint64_t hi,
                                    const char* path, mesa_noun gage);
IMPORT("store_commit") int host_store_commit(const uint8_t* buf, size_t len);

static mesa_engine* eng;

//  the store: libmesa's in-memory store, noting the keys each transaction
//  writes. A commit hands the host every row those keys now hold, as
//  [klen:4][key][vlen:4][val] (little-endian, vlen UINT32_MAX for a
//  deleted row), and the host saves them before it sends the datagrams
//  that commit released. store_load puts saved rows back.

static mesa_store* inn;     //  the memory store, which does the work
static mesa_store  sto;     //  it, as the engine sees it
static uint8_t*    dir_y;   //  keys written since begin, each [len:4][key]
static size_t      dir_z;
static size_t      dic_z;
static int         txn_i;

static int
_flush(void)
{
  size_t   cap_z = dir_z * 2 + 64;
  size_t   len_z = 0;
  uint8_t* out_y = malloc(cap_z);
  int      ret_i;

  for ( size_t at_z = 0; at_z < dir_z; ) {
    uint32_t       key_w;
    const uint8_t* key_y;
    uint8_t*       val_y = NULL;
    size_t         val_z = 0;
    uint32_t       len_w;
    int            got_i;

    memcpy(&key_w, dir_y + at_z, 4);
    key_y = dir_y + at_z + 4;
    at_z += 4 + key_w;

    got_i = inn->get(inn->ctx, key_y, key_w, &val_y, &val_z);
    if ( got_i < 0 ) {
      free(out_y);
      return -1;
    }
    if ( len_z + 8 + key_w + val_z > cap_z ) {
      cap_z = (len_z + 8 + key_w + val_z) * 2;
      out_y = realloc(out_y, cap_z);
    }
    len_w = got_i ? UINT32_MAX : (uint32_t)val_z;
    memcpy(out_y + len_z, &key_w, 4);
    memcpy(out_y + len_z + 4, key_y, key_w);
    memcpy(out_y + len_z + 4 + key_w, &len_w, 4);
    len_z += 8 + key_w;
    if ( !got_i ) {
      memcpy(out_y + len_z, val_y, val_z);
      len_z += val_z;
      mesa_free(val_y);
    }
  }
  dir_z = 0;
  ret_i = len_z ? host_store_commit(out_y, len_z) : 0;
  free(out_y);
  return ret_i;
}

static void
_dirty(const uint8_t* key_y, size_t key_z)
{
  uint32_t key_w = key_z;

  if ( dir_z + 4 + key_z > dic_z ) {
    dic_z = (dir_z + 4 + key_z) * 2;
    dir_y = realloc(dir_y, dic_z);
  }
  memcpy(dir_y + dir_z, &key_w, 4);
  memcpy(dir_y + dir_z + 4, key_y, key_z);
  dir_z += 4 + key_z;
}

static int
s_begin(void* ctx)
{
  int ret_i = inn->begin(inn->ctx);
  if ( !ret_i ) {
    txn_i = 1;
    dir_z = 0;
  }
  return ret_i;
}

static int
s_commit(void* ctx)
{
  int ret_i = inn->commit(inn->ctx);
  txn_i = 0;
  return ret_i ? ret_i : _flush();
}

static int
s_rollback(void* ctx)
{
  txn_i = 0;
  dir_z = 0;
  return inn->rollback(inn->ctx);
}

static int s_mark(void* ctx) { return inn->mark(inn->ctx); }
static int s_rewind(void* ctx) { return inn->rewind(inn->ctx); }
static int s_release(void* ctx) { return inn->release(inn->ctx); }

static int
s_get(void* ctx, const uint8_t* key_y, size_t key_z,
      uint8_t** val_y, size_t* val_z)
{
  return inn->get(inn->ctx, key_y, key_z, val_y, val_z);
}

//  a write outside a transaction is saved at once
static int
s_put(void* ctx, const uint8_t* key_y, size_t key_z,
      const uint8_t* val_y, size_t val_z)
{
  int ret_i = inn->put(inn->ctx, key_y, key_z, val_y, val_z);
  if ( !ret_i ) {
    _dirty(key_y, key_z);
    if ( !txn_i ) ret_i = _flush();
  }
  return ret_i;
}

static int
s_del(void* ctx, const uint8_t* key_y, size_t key_z)
{
  int ret_i = inn->del(inn->ctx, key_y, key_z);
  if ( !ret_i ) {
    _dirty(key_y, key_z);
    if ( !txn_i ) ret_i = _flush();
  }
  return ret_i;
}

static int
s_scan(void* ctx, const uint8_t* pre_y, size_t pre_z,
       mesa_store_scan_f fun_f, void* arg)
{
  return inn->scan(inn->ctx, pre_y, pre_z, fun_f, arg);
}

static void s_close(void* ctx) { inn->close(inn->ctx); }

static mesa_store*
_store(void)
{
  if ( !inn ) {
    inn = mesa_store_memory();
    sto = (mesa_store){
      .begin    = s_begin,
      .commit   = s_commit,
      .rollback = s_rollback,
      .mark     = s_mark,
      .rewind   = s_rewind,
      .release  = s_release,
      .get      = s_get,
      .put      = s_put,
      .del      = s_del,
      .scan     = s_scan,
      .close    = s_close,
    };
  }
  return &sto;
}

//  put back rows a commit handed out (deleted ones are skipped); before
//  open, and not journaled. -1 if [buf] is malformed.
EXPORT("store_load") int
u_store_load(const uint8_t* buf_y, size_t len_z)
{
  _store();
  for ( size_t at_z = 0; at_z < len_z; ) {
    uint32_t key_w;
    uint32_t val_w;

    if ( at_z + 4 > len_z ) return -1;
    memcpy(&key_w, buf_y + at_z, 4);
    if ( at_z + 8 + (size_t)key_w > len_z ) return -1;
    memcpy(&val_w, buf_y + at_z + 4 + key_w, 4);
    if ( UINT32_MAX == val_w ) {
      at_z += 8 + key_w;
      continue;
    }
    if ( at_z + 8 + (size_t)key_w + val_w > len_z ) return -1;
    inn->put(inn->ctx, buf_y + at_z + 4, key_w, buf_y + at_z + 8 + key_w, val_w);
    at_z += 8 + key_w + val_w;
  }
  return 0;
}

static mesa_ship
ship(uint64_t lo, uint64_t hi)
{
  return (mesa_ship){ .lo = lo, .hi = hi };
}

//  hooks

static void
h_send(void* ctx, mesa_lane lane, const uint8_t* buf, size_t len)
{
  host_send(lane.kind, lane.galaxy, lane.ip, lane.port, buf, len);
}

static void
h_need_point(void* ctx, mesa_ship who)
{
  host_need_point(who.lo, who.hi);
}

//  pokes go to the host; we run no agents, so watches are flubbed
static mesa_plea_result
h_plea(void* ctx, mesa_ship her, uint64_t bone, mesa_noun vane,
       mesa_noun path, mesa_noun payload, mesa_noun* error)
{
  mesa_gall_request req;

  switch ( mesa_gall_request_parse(vane, path, payload, &req) ) {
    case MESA_GALL_POKE:
      return host_on_poke(her.lo, her.hi, bone, req.agent, req.mark, req.noun);
    case MESA_GALL_WATCH:
    case MESA_GALL_WATCH_AS:
      return MESA_FLUB;
    case MESA_GALL_LEAVE:
      return MESA_ACK;
    default:
      return MESA_DROP;
  }
}

static void
h_boon(void* ctx, mesa_ship her, uint64_t bone, mesa_noun payload)
{
}

static void
h_done(void* ctx, mesa_ship her, uint64_t bone, mesa_noun error)
{
  host_on_done(her.lo, her.hi, bone, error);
}

static void
h_corked(void* ctx, mesa_ship her, uint64_t bone)
{
}

static void
h_sage(void* ctx, mesa_ship her, const char* path, mesa_noun gage)
{
  host_on_sage(her.lo, her.hi, path, gage);
}

static mesa_noun
h_peek(void* ctx, const mesa_ship* her, const char* path)
{
  return MESA_NONE;
}

static void
h_breach(void* ctx, mesa_ship her)
{
}

static uint64_t
h_shut_key(void* ctx, mesa_ship her, const char* agent, const char* spur)
{
  return 0;
}

//  memory and strings

EXPORT("alloc") void* u_alloc(size_t len) { return mesa_malloc(len); }
EXPORT("free") void u_free(void* ptr) { mesa_free(ptr); }

//  identity

//  mine a comet under one of the stars in [jam] (bootstrap.urbit.org's
//  comet-stars.jam), with 64 random bytes [eny]; its keyfile, or NULL
EXPORT("mine_comet") char*
u_mine_comet(const uint8_t* jam, size_t len, const uint8_t* eny)
{
  mesa_noun   lis = mesa_cue_bytes(len, jam);
  mesa_ship*  sar;
  mesa_ident  ide;
  size_t      n = 0;
  char*       key;

  if ( MESA_NONE == lis ) {
    return NULL;
  }
  for ( mesa_noun t = lis; mesa_is_cell(t); t = mesa_tail(t) ) {
    n++;
  }
  sar = mesa_malloc(n * sizeof(mesa_ship) + 1);
  n = 0;
  for ( mesa_noun t = lis; mesa_is_cell(t); t = mesa_tail(t) ) {
    uint64_t s;
    mesa_atom_get_u64(mesa_head(t), &s);
    sar[n++] = ship(s, 0);
  }
  mesa_unref(lis);

  key = mesa_comet_mine(n, sar, eny, UINT64_MAX, &ide)
      ? mesa_keyfile_render(&ide)
      : NULL;
  mesa_free(sar);
  return key;
}

EXPORT("ship_parse") int
u_ship_parse(const char* text, uint64_t* out)
{
  mesa_ship who;

  if ( !mesa_ship_parse(text, &who) ) {
    return 0;
  }
  out[0] = who.lo;
  out[1] = who.hi;
  return 1;
}

EXPORT("ship_render") char*
u_ship_render(uint64_t lo, uint64_t hi)
{
  return mesa_ship_render(ship(lo, hi));
}

//  what a keyfile says: its ship into out[0..1], its rift (UINT64_MAX if
//  the keyfile doesn't say) into out[2], its life into out[3]; 0 if it
//  isn't a keyfile
EXPORT("keyfile_ship") int
u_keyfile_ship(const char* keyfile, uint64_t* out)
{
  mesa_ident ide;

  if ( mesa_keyfile_parse(keyfile, &ide, 1) < 1 ) {
    return 0;
  }
  out[0] = ide.who.lo;
  out[1] = ide.who.hi;
  out[2] = ide.rift;
  out[3] = ide.life;
  return 1;
}

//  0 galaxy, 1 star, 2 planet, 3 moon, 4 comet
EXPORT("ship_rank") int
u_ship_rank(uint64_t lo, uint64_t hi)
{
  return mesa_ship_rank(ship(lo, hi));
}

//  a fake ship's keyfile (urbit -F keys, from its name), for tests that
//  must not spend a real identity
EXPORT("fake_keyfile") char*
u_fake_keyfile(uint64_t lo, uint64_t hi)
{
  mesa_ident ide;
  mesa_ident_fake(ship(lo, hi), &ide);
  return mesa_keyfile_render(&ide);
}

//  the engine

//  open as the comet or moon in [keyfile], on the store after any
//  store_load; a mesa_err. Comets may always start from an empty store; a
//  moon only when [breached] says it never ran (or was breached since).
//  Both have fixed sponsors.
EXPORT("open") int
u_open(const char* keyfile, uint64_t now, int breached)
{
  mesa_config cfg = { 0 };

  if ( mesa_keyfile_parse(keyfile, &cfg.ident, 1) < 1 ) {
    return MESA_E_ARG;
  }
  if ( MESA_RIFT_UNKNOWN == cfg.ident.rift ) {
    cfg.ident.rift = 0;
  }
  cfg.store = _store();
  cfg.hooks = (mesa_hooks){
    .send       = h_send,
    .need_point = h_need_point,
    .plea       = h_plea,
    .boon       = h_boon,
    .done       = h_done,
    .corked     = h_corked,
    .sage       = h_sage,
    .peek       = h_peek,
    .breach     = h_breach,
    .shut_key   = h_shut_key,
  };
  cfg.now = now;
  cfg.breached = breached;
  return mesa_engine_open(&cfg, &eng);
}

EXPORT("close") void
u_close(void)
{
  mesa_engine_close(eng);
  eng = NULL;
}

EXPORT("our_lo") uint64_t u_our_lo(void) { return mesa_engine_ship(eng).lo; }
EXPORT("our_hi") uint64_t u_our_hi(void) { return mesa_engine_ship(eng).hi; }

EXPORT("hear") void
u_hear(uint32_t ip, int port, const uint8_t* buf, size_t len, uint64_t now)
{
  mesa_lane lan = { .kind = MESA_LANE_IP, .ip = ip, .port = port };
  mesa_engine_hear(eng, lan, buf, len, now);
}

EXPORT("wake") void u_wake(uint64_t now) { mesa_engine_wake(eng, now); }
EXPORT("deadline") uint64_t u_deadline(void) { return mesa_engine_deadline(eng); }

EXPORT("point_request") char*
u_point_request(uint64_t lo, uint64_t hi)
{
  return mesa_azimuth_point_request(ship(lo, hi));
}

//  answer hooks.need_point with the roller's getPoint response [json];
//  a NULL [json] (the lookup failed) or a bad one reports no keys
EXPORT("point") int
u_point(uint64_t lo, uint64_t hi, const char* json, size_t len)
{
  mesa_point pot;

  if ( json
    && MESA_AZIMUTH_OK == mesa_azimuth_point_parse(ship(lo, hi), len, json, &pot) )
  {
    mesa_engine_point(eng, &pot);
    return 1;
  }
  mesa_engine_point_missing(eng, ship(lo, hi));
  return 0;
}

EXPORT("txn_begin") int u_txn_begin(void) { return mesa_txn_begin(eng); }
EXPORT("txn_commit") int u_txn_commit(void) { return mesa_txn_commit(eng); }
EXPORT("txn_abort") void u_txn_abort(void) { mesa_txn_abort(eng); }

//  a new forward flow to [lo hi]; its bone, or UINT64_MAX
EXPORT("flow_open") uint64_t
u_flow_open(uint64_t lo, uint64_t hi)
{
  uint64_t bone;
  return MESA_OK == mesa_flow_open(eng, ship(lo, hi), &bone) ? bone : UINT64_MAX;
}

EXPORT("poke") int
u_poke(uint64_t lo, uint64_t hi, uint64_t bone, const char* agent,
       const char* mark, mesa_noun noun)
{
  return mesa_gall_poke(eng, ship(lo, hi), bone, agent, mark, noun);
}

EXPORT("keen") int
u_keen(uint64_t lo, uint64_t hi, const char* path)
{
  return mesa_keen(eng, ship(lo, hi), path);
}

EXPORT("yawn") int
u_yawn(uint64_t lo, uint64_t hi, const char* path)
{
  return mesa_yawn(eng, ship(lo, hi), path);
}

//  nouns

EXPORT("atom") mesa_noun
u_atom(const uint8_t* buf, size_t len)
{
  return mesa_atom_bytes(len, buf);
}

EXPORT("cell") mesa_noun u_cell(mesa_noun h, mesa_noun t) { return mesa_cell(h, t); }
EXPORT("is_cell") int u_is_cell(mesa_noun n) { return mesa_is_cell(n); }
EXPORT("head") mesa_noun u_head(mesa_noun n) { return mesa_head(n); }
EXPORT("tail") mesa_noun u_tail(mesa_noun n) { return mesa_tail(n); }
EXPORT("met3") size_t u_met3(mesa_noun n) { return mesa_met3(n); }
EXPORT("ref") mesa_noun u_ref(mesa_noun n) { return mesa_ref(n); }
EXPORT("unref") void u_unref(mesa_noun n) { mesa_unref(n); }

EXPORT("atom_bytes") void
u_atom_bytes(mesa_noun n, uint8_t* buf, size_t len)
{
  mesa_atom_get_bytes(n, len, buf);
}
