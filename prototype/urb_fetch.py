"""urb_fetch: read a lattice page by its urb:// address, with no ship.

  LIBMESA=.../libmesa.so LIBMESA_PY=.../bindings/python \\
    python3 -I urb_fetch.py [--moon KEYFILE [--new-moon]] [--state DIR] [-o DIR] URL...

Runs as a comet (mined once, kept in --state) or as a moon from its |moon
keyfile. For each URL it mirrors lattice's own reader (nex/lattice/app.hoon
+read-page-or-index and +read-page-body-rev):

  1. resolve the url as +de-urb does (only the published forms: bare, /n/)
  2. read the publisher's /pub/revs over grubbery's cross-ship peek
     (%grubbery-load poke, %grubbery-transfer back), cached for 60 s
  3. %keen the page body at /g/x/1/grubbery//1/pub/page/<rel>/<id>
  4. if the list has no entry or the keen misses, peek the vault file

One URL prints its body on stdout. With -o, every body is written there.
"""
import argparse, os, re, sys, time, urllib.request

sys.path.insert(0, os.environ.get("LIBMESA_PY", "."))
import libmesa as M

DESK_BASE = "/apps/shell.shell/desks/lattice.desk/desk/data/lattice.lattice_app"
APP_BASE = "/apps/lattice.lattice_app"
REVS_TTL = 60        # +revs-ttl
KEEN_TIMEOUT = 10    # +mesa-timeout
PEEK_TIMEOUT = 30    # +remote-timeout
KNOT = re.compile(r"^[a-z0-9~._-]+$")


class UrbError(Exception):
    pass


# ------------------------------------------------------------- addresses

def de_urb(url):
    """[ship, rel] for a published page's url, as lattice's +de-urb resolves
    it. /p/, /k/ and /t/ name the publisher's tree, which needs its grants
    and is not handled here."""
    if not url.startswith("urb://"):
        raise UrbError("not an urb:// url")
    who, _, rest = url[6:].partition("/")
    try:
        M.ship(who)
    except ValueError:
        raise UrbError("no ship in %s" % url)
    rest = rest[:-1] if rest.endswith("/") else rest   # urb://~s/site/ = urb://~s/site
    if not rest:
        return who, ["index"]
    segs = rest.split("/")
    if not all(KNOT.match(s) for s in segs):
        raise UrbError("bad path in %s" % url)
    if len(segs[0]) > 1:
        return who, segs                                # frozen legacy form
    if segs[0] == "n":
        return who, segs[1:]
    if segs[0] in ("p", "k", "t"):
        raise UrbError("/%s/ addresses read the publisher's tree: not supported" % segs[0])
    raise UrbError("/%s/ is not a mount" % segs[0])


def looks_html(body):
    """lattice-gmi's +looks-html: the first non-blank character is <"""
    return body[:256].lstrip(" \t\r\n").startswith("<")


# ------------------------------------------------------------- nouns

def entries(n):
    """the nodes of a hoon map (a treap of [n l r])"""
    out, stack = [], [n]
    while stack:
        t = stack.pop()
        if t.is_cell:
            out.append(t.head)
            stack += [t.tail.head, t.tail.tail]
    return out


def unit(n):
    return n.tail if n.is_cell else None


def js(n):
    """a $json noun as Python"""
    if not n.is_cell:
        return None
    tag, p = n.head.text(), n.tail
    if tag == "o":
        return {kv.head.text(): js(kv.tail) for kv in entries(p)}
    if tag == "a":
        out = []
        while p.is_cell:
            out.append(js(p.head))
            p = p.tail
        return out
    if tag == "b":
        return int(p) == 0
    if tag == "n":
        return float(p.text())
    return p.text()


def file_of(snap, silo, conv):
    """the file a %snap's pace names, out of its inline or %data silo"""
    pace = snap.tail.head
    jobe = unit(pace.tail) if pace.head.text() in ("firm", "temp") else None
    if jobe is None:
        return None
    jects = {int(e.head): e.tail.tail for e in entries(silo.tail)}
    nouns = {int(e.head): e.tail.tail for e in entries(silo.head)}
    ject = jects.get(int(jobe))
    if ject is None or ject.head.text() != "leaf":
        return None
    return conv(nouns[int(ject.tail.head)])    # convert now: nouns are borrowed


# ------------------------------------------------------------- the reader

class Reader(M.Node):
    """a libmesa node that reads published pages; calls block on the loop"""

    def setup(self):
        self.peeks = {}       # wire -> state of one peek
        self.keens = {}       # scry path -> (mark, body) once answered
        self.flows = {}       # ship -> bone of our flow to its %grubbery
        self.bases = {}       # ship -> where it runs lattice
        self.revs = {}        # ship -> (fetched at, {page: {rev, id}})
        #  wires never repeat across runs: a publisher can deliver a reply
        #  meant for an earlier process late, and it must not match ours
        self.nonce = os.urandom(4).hex()
        self.n = 0

    def wait(self, done, secs):
        end = time.time() + secs
        while not done() and time.time() < end:
            self.step(0.1)
        return done()

    # grubbery's cross-ship peek of one file: [dest-dir, name]
    def peek(self, her, folder, name, conv):
        self.n += 1
        wire = "/urb-fetch/%s/%d" % (self.nonce, self.n)
        st = self.peeks[wire] = {"conv": conv, "done": False, "val": None}
        with self.txn():
            if her not in self.flows:
                self.flows[her] = self.open_flow(M.ship(her))
            st["bone"] = self.flows[her]
            self.poke(M.ship(her), st["bone"], "grubbery", "grubbery-load",
                      ((M.path(wire), (True, M.path(folder), name)), ("peek", None, True)))
        self.wait(lambda: st["done"], PEEK_TIMEOUT)
        del self.peeks[wire]
        return st["val"]        # None: refused, nothing there, or no answer

    def on_poke(self, her, agent, mark, n):
        if agent != "grubbery" or mark != "grubbery-transfer":
            return M.ACK
        wire = "/" + "/".join(s.text() for s in self._list(n.head))
        tag, r = n.tail.head.text(), n.tail.tail
        if wire == "/want":
            #  +process-want answers on the fixed wire /want, not ours, so
            #  match the content to the snap that asked for it, as
            #  grubbery's own reader does
            wants = [st for st in self.peeks.values() if "snap" in st and not st["done"]]
            for st in wants:
                v = file_of(st["snap"], r, st["conv"]) if tag == "data" else None
                if v is not None:
                    st["val"], st["done"] = v, True
            if tag == "miss" and wants:      # the pin expired before our %want
                wants[0]["done"] = True
            return M.ACK
        st = self.peeks.get(wire)
        if st is None:
            return M.ACK
        if tag == "snap":
            snap, data = unit(r.tail.tail.head), unit(r.tail.tail.tail)
            if snap is not None and data is None:
                #  too big to inline: ask for the content, pinned for 5 minutes
                st["snap"] = M.Noun(M.lib.mesa_ref(snap.n))
                self.poke(her, st["bone"], "grubbery", "grubbery-transfer",
                          (n.head, "want", r.head, int(r.tail.head)))
                return M.ACK
            st["val"] = snap and file_of(snap, data, st["conv"])
        st["done"] = True
        return M.ACK

    def on_done(self, her, bone, error):
        #  a nacked request: report it and stop waiting on that ship's peeks
        if error is not None:
            print("nack from %s: %s" % (M.ship_text(her), M.tang(error)), file=sys.stderr)
            for st in self.peeks.values():
                if st["bone"] == bone:
                    st["done"] = True

    @staticmethod
    def _list(n):
        while n.is_cell:
            yield n.head
            n = n.tail

    def keen(self, her, spath):
        with self.txn():
            self.scry(M.ship(her), spath)
        if not self.wait(lambda: spath in self.keens, KEEN_TIMEOUT):
            #  a miss never answers: stop asking, as +keen-page-raw %yawns
            with self.txn():
                self.unscry(M.ship(her), spath)
            return None
        return self.keens.pop(spath)

    def on_scry(self, her, path_, mark, n):
        self.keens[path_] = None if mark is None else (mark, n.text() if not n.is_cell else None)

    # lattice's reader, one layer at a time
    def base(self, her):
        """+peer-base: where the peer runs lattice, asked, not guessed"""
        if her not in self.bases:
            for b in (DESK_BASE, APP_BASE):
                got = self.peek(her, b + "/pub", "revs", js)
                if got is not None:
                    self.bases[her] = b
                    self.revs[her] = (time.time(), got)
                    break
            else:
                raise UrbError("%s answered no lattice peek (offline, or no lattice)" % her)
        return self.bases[her]

    def rev_list(self, her):
        """+peer-revs-cached"""
        b = self.base(her)
        at, revs = self.revs.get(her, (0, None))
        if revs is None or time.time() - at > REVS_TTL:
            revs = self.peek(her, b + "/pub", "revs", js) or {}
            self.revs[her] = (time.time(), revs)
        return revs

    def page(self, her, rel):
        """+read-page-body-rev: keen the listed id, else peek the vault file"""
        key = "/" + "/".join(rel)
        ent = self.rev_list(her).get(key)
        if ent:
            got = self.keen(her, "/g/x/1/grubbery//1/pub/page%s/%s" % (key, ent["id"]))
            if got and got[0] == "gmi" and got[1] is not None:
                return got[1], "keen"
            self.revs.pop(her, None)       # a miss means the list is stale
        body = self.peek(her, self.base(her) + "/pub/vault" + key, "gmi",
                         lambda n: n.text() if not n.is_cell else None)
        return (body, "peek") if body is not None else (None, None)

    def fetch(self, url):
        """+read-page-or-index for one url: (key, body, how)"""
        her, rel = de_urb(url)
        revs = self.rev_list(her)
        hit = lambda p: ("/" + "/".join(p)) in revs
        dex = rel + ["index"]
        if not rel or rel[-1] == "index":
            tries = [rel]
        elif not hit(rel) and hit(dex):
            tries = [dex]
        elif revs and not hit(rel):
            tries = [rel]
        else:
            tries = [rel, dex]
        for p in tries:
            body, how = self.page(her, p)
            if body is not None:
                return "/" + "/".join(p), body, how
        raise UrbError("%s: not published" % url)


# ------------------------------------------------------------- the CLI

def identity(args):
    """the comet in --state (mined on first use), or the moon's keyfile"""
    if args.moon:
        ide = M.Identity.keyfile(args.moon)
        db = os.path.join(args.state, M.ship_text(ide.ship).lstrip("~") + ".db")
        if not os.path.exists(db) and not args.new_moon:
            raise UrbError("no state for this moon in %s. Pass --new-moon only if it has "
                           "never run anywhere (not booted, not used by another copy); "
                           "otherwise breach it first (|moon-breach on its parent)" % args.state)
        return ide, db, args.new_moon and not os.path.exists(db)
    #  bootstrap.urbit.org answers 403 to urllib's default User-Agent
    op = urllib.request.build_opener()
    op.addheaders = [("User-Agent", "urb-fetch")]
    urllib.request.install_opener(op)
    ide = M.Identity.comet(os.path.join(args.state, "comet.key"))
    return ide, os.path.join(args.state, "comet.db"), False


def port(db):
    """one UDP port per identity (beside its db), kept across runs. Publishers send their
    replies to the address they last saw: a new port each run leaves them
    resending to a dead one, with newer replies queued behind it (a 79 s
    stall, measured)."""
    f = db + ".port"
    if not os.path.exists(f):
        with open(f, "w") as o:
            o.write(str(20000 + int.from_bytes(os.urandom(2), "big") % 40000))
    return int(open(f).read())


def main():
    ap = argparse.ArgumentParser(description="read lattice pages by urb:// address")
    ap.add_argument("url", nargs="+")
    ap.add_argument("--moon", help="run as this moon (its |moon keyfile)")
    ap.add_argument("--new-moon", action="store_true",
                    help="the moon has never run anywhere: start its state fresh")
    ap.add_argument("--state", default="urb-state", help="identity and network state")
    ap.add_argument("-o", "--out", help="write each body to this directory")
    args = ap.parse_args()
    os.makedirs(args.state, mode=0o700, exist_ok=True)

    t0 = time.time()
    ide, db, fresh = identity(args)
    node = Reader(ide, db, breached=fresh, port=port(db))
    print("as %s (ready in %.1fs)" % (M.ship_text(node.our), time.time() - t0), file=sys.stderr)
    ok = True
    for url in args.url:
        t = time.time()
        try:
            key, body, how = node.fetch(url)
        except UrbError as e:
            print("FAIL %s  %.1fs" % (e, time.time() - t), file=sys.stderr)
            ok = False
            continue
        kind = "html" if looks_html(body) else "gemtext"
        print("ok   %s -> %s  %s, %d bytes, by %s, %.1fs"
              % (url, key, kind, len(body.encode()), how, time.time() - t), file=sys.stderr)
        if args.out:
            os.makedirs(args.out, exist_ok=True)
            name = url[6:].strip("/").replace("/", "_") + (".html" if kind == "html" else ".gmi")
            with open(os.path.join(args.out, name), "w") as f:
                f.write(body)
        elif len(args.url) == 1:
            sys.stdout.write(body)
    node.close()
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
