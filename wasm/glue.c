//  glue.c: libmesa's engine as a wasm module for a JavaScript host.
//
//  The engine's structs (ships, lanes, points, identities) stay in C. A
//  ship crosses to JavaScript as two 64-bit halves, a noun as its 64-bit
//  handle, a string as a pointer to NUL-terminated UTF-8. The hooks call
//  the host's imports, and the host calls the exports below. One engine
//  per module, run by a comet on an in-memory store.

#include <stdint.h>
#include <stdlib.h>

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

static mesa_engine* eng;

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

//  the engine

//  open as the comet in [keyfile]; a mesa_err. Comets may always start
//  from an empty store, and their sponsor is fixed.
EXPORT("open") int
u_open(const char* keyfile, uint64_t now)
{
  mesa_config cfg = { 0 };

  if ( mesa_keyfile_parse(keyfile, &cfg.ident, 1) < 1 ) {
    return MESA_E_ARG;
  }
  if ( MESA_RIFT_UNKNOWN == cfg.ident.rift ) {
    cfg.ident.rift = 0;
  }
  cfg.store = mesa_store_memory();
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
