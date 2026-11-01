// The lattice reader against a publisher that answers late or not at
// all, on a mocked clock: each case is a difference the Go gateway's
// reader (reader.go) got right and this one got wrong.

import assert from 'node:assert/strict';
import { mock, test } from 'node:test';
import { UrbError } from '../mesa.mjs';
import { victim } from './fleet.mjs';

const T0 = 1_761_600_000_000;

// a promise's state, as it stands
const state = p => {
  const s = { done: false };
  p.then(v => Object.assign(s, { done: true, v }), e => Object.assign(s, { done: true, e }));
  return s;
};

// a victim engine on mocked timers and a mocked Date: wait(ms) moves
// both on, letting what is due run
async function setup() {
  mock.timers.enable({ apis: ['setTimeout', 'Date'], now: T0 });
  const flush = async () => {
    for (let i = 0; i < 20; i++) await new Promise(r => setImmediate(r));
  };
  //  open() waits on a timer of its own
  const up = state(victim({ now: () => Date.now() }));
  while (!up.done) {
    await new Promise(r => setImmediate(r));
    mock.timers.tick(1);
  }
  if (up.e) throw up.e;
  const v = up.v;
  const wait = async ms => {
    for (let t = 0; t < ms; t += 500) {
      mock.timers.tick(Math.min(500, ms - t));
      await flush();
    }
  };
  const wire = n => v.path(`/urb-fetch/${v.nonce}/${n}`);
  // ~nec answers our peek n with a file holding value
  const file = (n, value) => v.deliver('grubbery-transfer', [wire(n), 'snap', null, 5,
    [null, [null, ['firm', null, 7], null]],
    [null, [[[9, null, value], null, null], [[7, null, 'leaf', 9, null], null, null]]]]);
  // ... or with nothing there
  const veto = n => v.deliver('grubbery-transfer', [wire(n), 'veto', null]);
  // a revision list as $json: {"/page": {"rev": 1, "id": id}}, an id of
  // undefined left out
  const treap = items => (items.length ? [items[0], treap(items.slice(1)), null] : null);
  const revs = list => ['o', treap(Object.entries(list).map(([k, id]) =>
    [k, ['o', treap([['rev', ['n', '1']], ...(id === undefined ? [] : [['id', ['s', id]]])])]]))];
  const keen = (page, id) => `/g/x/1/grubbery//1/pub/page${page}/${id}`;
  const done = async () => {
    const closed = state(v.f.close());
    while (!closed.done) {
      await new Promise(r => setImmediate(r));
      mock.timers.tick(100);
    }
    mock.timers.reset();
  };
  return { v, flush, wait, state, file, veto, revs, keen, done };
}

test('a base probe nobody answers ends the read at 30 s', async () => {
  const t = await setup();
  try {
    const r = t.state(t.v.urb.read('urb://~nec/page'));
    await t.wait(30_500);
    //  the app base would only wait behind the same stall: no second
    //  probe, and later, since a ship catching up answers in a minute
    assert.ok(r.done, 'still waiting at 30 s');
    assert.ok(r.e instanceof UrbError && r.e.later === true && /~nec answered no lattice peek/.test(r.e.message), String(r.e));
    //  a ship that answers both probes with nothing runs no lattice: not
    //  later
    const n = t.state(t.v.urb.read('urb://~nec/page'));
    await t.flush();
    t.veto(2);
    await t.flush();
    t.veto(3);
    await t.flush();
    assert.ok(n.done && n.e instanceof UrbError && !n.e.later && /answered no lattice peek/.test(n.e.message), String(n.e));
  } finally {
    await t.done();
  }
});

test('a list refresh nobody answers keeps the stale list', async () => {
  const t = await setup();
  try {
    const first = t.state(t.v.urb.read('urb://~nec/md'));
    await t.flush();
    t.file(1, t.revs({ '/md': '0v1' }));
    await t.flush();
    t.v.sage(t.keen('/md', '0v1'), ['md', '# hi']);
    await t.flush();
    assert.equal(first.v?.mark, 'md', String(first.e));
    //  past the list's 60 s, a refresh nobody answers
    await t.wait(61_000);
    const again = t.state(t.v.urb.read('urb://~nec/md'));
    await t.wait(30_500);
    //  read by the stale list's id, so still markdown, not by the vault
    //  peek as gemtext
    t.v.sage(t.keen('/md', '0v1'), ['md', '# hi']);
    t.file(3, '# hi');
    await t.flush();
    assert.ok(again.done, 'still waiting');
    assert.equal(again.v?.mark, 'md', String(again.e));
    assert.equal(again.v.how, 'keen');
  } finally {
    await t.done();
  }
});

test('one read looks each page up in the one list it got', async () => {
  const t = await setup();
  try {
    const r = t.state(t.v.urb.read('urb://~nec/aa'));
    await t.flush();
    t.file(1, t.revs({ '/aa': '0v1' }));
    await t.flush();
    //  the listed id misses, and the vault has no /aa
    t.v.sage(t.keen('/aa', '0v1'), null);
    await t.flush();
    t.veto(2);
    await t.flush();
    //  so /aa/index, from the vault, with no second list asked for
    t.file(3, 'the index');
    await t.flush();
    assert.ok(r.done, 'still waiting: the index try asked for the list again');
    assert.deepEqual([r.v?.key, r.v?.body, r.v?.how], ['/aa/index', 'the index', 'peek'], String(r.e));
  } finally {
    await t.done();
  }
});

test('a list that answers after its read gave up is kept', async () => {
  const t = await setup();
  try {
    const r = t.state(t.v.urb.read('urb://~nec/md'));
    await t.wait(30_500);
    assert.ok(r.done && r.e instanceof UrbError, String(r.e));
    //  it comes in at 40 s, under 5 minutes late
    await t.wait(10_000);
    t.file(1, t.revs({ '/md': '0v1' }));
    await t.flush();
    assert.ok(t.v.logs.some(l => /a late answer came in on/.test(l)), 'logged');
    //  so the next read keens the page at once
    const again = t.state(t.v.urb.read('urb://~nec/md'));
    await t.flush();
    t.v.sage(t.keen('/md', '0v1'), ['md', '# hi']);
    await t.flush();
    assert.ok(again.done, 'still waiting: the late list was dropped');
    assert.equal(again.v?.mark, 'md', String(again.e));
    //  one more than 5 minutes late is dropped, once a peek after them
    //  has cleared it
    await t.wait(61_000);
    const third = t.state(t.v.urb.read('urb://~nec/md'));
    await t.wait(30_500);
    t.v.sage(t.keen('/md', '0v1'), ['md', '# hi']);
    await t.flush();
    assert.equal(third.v?.how, 'keen', String(third.e));
    await t.wait(300_000);
    const fourth = t.state(t.v.urb.read('urb://~nec/md'));
    await t.flush();
    t.file(2, t.revs({ '/md': '0v2' }));
    await t.wait(30_500);
    t.v.sage(t.keen('/md', '0v1'), ['md', '# old']);
    t.v.sage(t.keen('/md', '0v2'), ['md', '# new']);
    await t.flush();
    assert.equal(fourth.v?.body, '# old', String(fourth.e));
  } finally {
    await t.done();
  }
});

test('a list entry with no id is no entry', async () => {
  const t = await setup();
  try {
    const r = t.state(t.v.urb.read('urb://~nec/xx'));
    await t.flush();
    t.file(1, t.revs({ '/xx': undefined, '/yy': '' }));
    await t.flush();
    //  nothing to keen: /xx by the vault peek, at once
    t.file(2, 'from the vault');
    await t.flush();
    assert.ok(r.done, 'still waiting: it keened /xx/undefined');
    assert.deepEqual([r.v?.body, r.v?.how], ['from the vault', 'peek'], String(r.e));
  } finally {
    await t.done();
  }
});
