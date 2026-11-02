//! furum.mjs and forum.mjs on the core: their calls, each with its
//! awaits where the JS has them, so their results come in the JS's
//! order (see [`crate::rt`]). What they read is [`crate::forum`]'s.

use super::*;
use crate::forum::{board, furum as fu};
use std::any::Any;

/// furum(urb, host)'s calls
#[derive(Clone, Debug)]
pub enum FurumCall {
    NextPost(String),
    Posts(String),
    Comments(String, f64),
    Votes(String, f64),
    /// newPost(name, {title, url, body})
    NewPost {
        name: String,
        title: String,
        url: Option<String>,
        body: Option<String>,
    },
    /// newComment(name, postId, body, parent)
    NewComment {
        name: String,
        post: f64,
        body: String,
        parent: Option<f64>,
    },
    Upvote(String, f64),
    /// onNote: each note furum sends us is an [`Output::Note`]
    /// `furum.note`
    OnNote,
}

/// forum(urb, host)'s calls
#[derive(Clone, Debug)]
pub enum ForumCall {
    Board(String),
    Thread(String, msg::Id),
    NewPost {
        name: String,
        title: Option<String>,
        url: Option<String>,
        body: Option<String>,
    },
    NewComment {
        name: String,
        post: msg::Id,
        body: Option<String>,
        parent: Option<msg::Id>,
    },
    Upvote(String, msg::Target),
    Downvote(String, msg::Target),
    Unvote(String, msg::Target),
    /// onNote: each refusal note the host sends us is an
    /// [`Output::Note`] `forum.note`
    OnNote,
}

/// who hears the grubbery pokes other ships send ours (grub.onInbox's f)
#[derive(Clone, Debug)]
pub(crate) enum Listener {
    /// each is an [`Output::Inbox`]
    Emit,
    Furum,
    /// forum(urb, host)'s
    Forum(String),
}

/// one forum(urb, host): the host's base, once it answered
#[derive(Default)]
pub(crate) struct ForumSt {
    base: Cell<Option<&'static str>>,
}

fn any(v: impl Any) -> Val {
    Val::Any(Rc::new(v))
}

fn js(v: Js) -> Val {
    Val::Js(Rc::new(v))
}

fn file_conv(f: impl for<'a> Fn(Measured<'a>) -> Result<Val, Failure> + 'static) -> Read {
    Read {
        dir: false,
        conv: Conv2::File(Rc::new(f)),
    }
}

/// noun() of a number: BigInt(x), or its RangeError
fn number(x: f64) -> Result<Value<'static>, Failure> {
    js::bigint_of_number(x).map(Value::Int)
}

/// furum.mjs's some: a truthy value, or null
fn some_text(v: Option<String>) -> Value<'static> {
    match v {
        Some(s) if !s.is_empty() => Value::Array(vec![Value::Null, s.into()]),
        _ => Value::Null,
    }
}

/// newComment's parent: none only when absent, as comment ids start
/// at 0 (`parent == null ? null : [null, parent]`)
fn some_parent(parent: Option<f64>) -> Result<Value<'static>, Failure> {
    match parent {
        Some(p) => number(p).map(|p| Value::Array(vec![Value::Null, p])),
        None => Ok(Value::Null),
    }
}

impl World {
    /// trace.mjs's forumOf(host)
    fn forum_of(&self, host: &str) -> Rc<ForumSt> {
        self.forums
            .borrow_mut()
            .entry(host.to_string())
            .or_default()
            .clone()
    }

    /// a furum call: its promise, or what it threw at once
    pub(super) fn furum(&self, host: String, call: FurumCall) -> Result<Val, Failure> {
        let i = self.urb("nouns")?;
        let board = |name: &str| format!("{}/boards/{name}", fu::BASE);
        // act(...action): a poke of our [%act action] to the host's inbox
        let act = |w: &World, action: Vec<Result<Value<'static>, Failure>>| {
            let her = ship_of(&host)?;
            let mut v = vec![Ok(Value::from("act"))];
            v.extend(action);
            let value = v
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array);
            Ok(Val::Promise(grub_poke_with(
                w,
                &i,
                her,
                fu::BASE,
                "inbox.sig",
                (fu::MSG.0.into(), fu::MSG.1.into()),
                value,
            )))
        };
        match call {
            FurumCall::NextPost(name) => Ok(Val::Promise(next_post(
                self,
                &i,
                ship_of(&host)?,
                &board(&name),
            ))),
            FurumCall::Posts(name) => {
                let i = i.clone();
                Ok(Val::Promise(spawn(self, move |w| {
                    let her = match ship_of(&host) {
                        Ok(h) => h,
                        Err(e) => return throw(e),
                    };
                    let p = next_post(w, &i, her, &board(&name));
                    wait_ok(Val::Promise(p), move |w, next| {
                        let next = match next {
                            Val::Js(j) => match *j {
                                Js::Num(n) => n,
                                _ => return throw(Failure::error("no next post")),
                            },
                            _ => {
                                return throw(Failure::error(format!(
                                    "no board {name} on {host}, or it is not shared"
                                )));
                            }
                        };
                        if next > fu::MAX_POSTS {
                            return throw(Failure::urb(format!(
                                "{host}'s board {name} claims {} posts, over 100000",
                                js::number_text(next)
                            )));
                        }
                        posts(w, i, her, board(&name), next, 0, Vec::new())
                    })
                })))
            }
            FurumCall::Comments(name, id) => {
                let i = i.clone();
                Ok(Val::Promise(spawn(self, move |w| {
                    let her = match ship_of(&host) {
                        Ok(h) => h,
                        Err(e) => return throw(e),
                    };
                    let read = file_conv(move |n| Ok(any(fu::comments(n, id)?)));
                    let p = peek(
                        w,
                        &i,
                        her,
                        &format!("{}/content/threads", board(&name)),
                        Some(&bucket(id)),
                        read,
                        PEEK_TIMEOUT,
                    );
                    wait_ok(Val::Promise(p), |_, got| {
                        let mut cs = match got {
                            Val::Any(a) => a
                                .downcast_ref::<Option<Vec<fu::Comment>>>()
                                .cloned()
                                .flatten()
                                .unwrap_or_default(),
                            _ => Vec::new(),
                        };
                        fu::sort_comments(&mut cs);
                        Step::Ret(js(Js::Arr(cs.iter().map(fu::Comment::js).collect())))
                    })
                })))
            }
            FurumCall::Votes(name, id) => {
                let i = i.clone();
                Ok(Val::Promise(spawn(self, move |w| {
                    let her = match ship_of(&host) {
                        Ok(h) => h,
                        Err(e) => return throw(e),
                    };
                    let read = file_conv(move |n| {
                        Ok(match fu::votes(n, id)? {
                            Some(t) => any(t),
                            None => Val::Null,
                        })
                    });
                    let p = peek(
                        w,
                        &i,
                        her,
                        &format!("{}/content/votes", board(&name)),
                        Some(&bucket(id)),
                        read,
                        PEEK_TIMEOUT,
                    );
                    wait_ok(Val::Promise(p), |_, got| {
                        let (up, down) = match got {
                            Val::Any(a) => a
                                .downcast_ref::<(Vec<String>, Vec<String>)>()
                                .cloned()
                                .unwrap_or_default(),
                            _ => Default::default(),
                        };
                        Step::Ret(js(fu::votes_js(&up, &down)))
                    })
                })))
            }
            FurumCall::NewPost {
                name,
                title,
                url,
                body,
            } => act(
                self,
                vec![
                    Ok("new-post".into()),
                    Ok(name.into()),
                    Ok(title.into()),
                    Ok(some_text(url)),
                    Ok(some_text(body)),
                ],
            ),
            FurumCall::NewComment {
                name,
                post,
                body,
                parent,
            } => act(
                self,
                vec![
                    Ok("new-comment".into()),
                    Ok(name.into()),
                    number(post),
                    some_parent(parent),
                    Ok(body.into()),
                ],
            ),
            FurumCall::Upvote(name, post) => act(
                self,
                vec![
                    Ok("upvote".into()),
                    Ok(name.into()),
                    number(post).map(|p| Value::Array(vec!["post".into(), p])),
                ],
            ),
            FurumCall::OnNote => {
                *i.inbox.borrow_mut() = Some(Listener::Furum);
                Ok(Val::Reply(Reply::True))
            }
        }
    }

    /// a forum call: its promise, or what it threw at once
    pub(super) fn forum(&self, host: String, call: ForumCall) -> Result<Val, Failure> {
        let i = self.urb("nouns")?;
        let st = self.forum_of(&host);
        let act = |w: &World, action: Result<Value<'static>, UrbError>| -> Result<Val, Failure> {
            let action = action?;
            let hb = host_base(w, &i, &st, &host);
            let (i, host) = (i.clone(), host.clone());
            Ok(Val::Promise(on_ok(w, &hb, move |w, b| {
                let Val::Str(b) = b else {
                    return Err(Val::Err(Failure::error("no base")));
                };
                let her = ship_of(&host).map_err(Val::Err)?;
                Ok(Val::Promise(grub_poke_with(
                    w,
                    &i,
                    her,
                    &b,
                    "forum.sig",
                    (board::ACTION.0.into(), board::ACTION.1.into()),
                    Ok(action),
                )))
            })))
        };
        let name_of = |name: &str| -> Result<(), Failure> {
            if !board::is_name(name) {
                return Err(Failure::urb(format!("not a board name: {name}")));
            }
            Ok(())
        };
        match call {
            ForumCall::Board(name) => Ok(Val::Promise(board_of(self, &i, &st, host, name))),
            ForumCall::Thread(name, post) => {
                let (i, st) = (i.clone(), st.clone());
                Ok(Val::Promise(spawn(self, move |w| {
                    let post = match msg::id_of(&post) {
                        Ok(p) => p,
                        Err(e) => return throw(e.into()),
                    };
                    let p = board_of(w, &i, &st, host.clone(), name.clone());
                    wait_ok(Val::Promise(p), move |_, b| {
                        let Some(b) = b.any::<board::Board>() else {
                            return throw(Failure::error("no board"));
                        };
                        match b.thread(&host, &name, &post) {
                            Ok(t) => Step::Ret(js(t)),
                            Err(e) => throw(e.into()),
                        }
                    })
                })))
            }
            ForumCall::NewPost {
                name,
                title,
                url,
                body,
            } => {
                name_of(&name)?;
                act(
                    self,
                    msg::forum::new_post(&name, title.as_deref(), url.as_deref(), body.as_deref()),
                )
            }
            ForumCall::NewComment {
                name,
                post,
                body,
                parent,
            } => {
                name_of(&name)?;
                act(
                    self,
                    msg::forum::new_comment(&name, &post, body.as_deref(), parent.as_ref()),
                )
            }
            ForumCall::Upvote(name, t) => {
                name_of(&name)?;
                act(self, msg::forum::upvote(&name, &t))
            }
            ForumCall::Downvote(name, t) => {
                name_of(&name)?;
                act(self, msg::forum::downvote(&name, &t))
            }
            ForumCall::Unvote(name, t) => {
                name_of(&name)?;
                act(self, msg::forum::unvote(&name, &t))
            }
            ForumCall::OnNote => {
                *i.inbox.borrow_mut() = Some(Listener::Forum(host));
                Ok(Val::Reply(Reply::True))
            }
        }
    }

    /// grub.onInbox's f, during the hook: an app's note, or the poke
    pub(super) fn hear_inbox(&self, l: &Listener, msg: Inbox, noun: Measured<'_>) {
        let got = match l {
            Listener::Emit => {
                self.emit(Output::Inbox(msg));
                return;
            }
            Listener::Furum => fu::note((&msg.mark.0, &msg.mark.1), noun).map(|n| {
                n.map(|(title, body)| {
                    (
                        "furum.note",
                        Js::obj([
                            ("from", Js::Str(msg.from.clone())),
                            ("title", Js::Str(title)),
                            ("body", Js::Str(body)),
                        ]),
                    )
                })
            }),
            Listener::Forum(host) => board::note(
                &msg.from,
                host,
                (&msg.to.0, &msg.to.1),
                (&msg.mark.0, &msg.mark.1),
                noun,
            )
            .map(|n| {
                n.map(|(b, reason)| {
                    (
                        "forum.note",
                        Js::obj([
                            ("host", Js::Str(msg.from.clone())),
                            ("board", Js::Str(b)),
                            ("reason", Js::Str(reason)),
                            ("at", Js::Date(Some(self.now() as i64))),
                        ]),
                    )
                })
            }),
        };
        match got {
            Ok(Some((name, value))) => self.emit(Output::Note {
                name: name.into(),
                value,
            }),
            Ok(None) => {}
            // inbox(msg) threw: logged
            Err(e) => self.log(Failure::from(e).log_text()),
        }
    }
}

impl Val {
    fn any<T: 'static>(&self) -> Option<&T> {
        match self {
            Val::Any(a) => a.downcast_ref(),
            _ => None,
        }
    }
}

/// a bucket's name: `b<id / 100>`, as the JS writes it
fn bucket(id: f64) -> String {
    format!("b{}", js::number_text((id / 100.0).floor()))
}

/// nextPost(name): the id the board's next post gets, or null
fn next_post(w: &World, i: &Rc<Inst>, her: Ship, board: &str) -> Promise {
    let read = file_conv(|n| Ok(fu::next_post(n)?.map_or(Val::Null, |n| js(Js::Num(n)))));
    peek(
        w,
        i,
        her,
        &format!("{board}/pub"),
        Some("conf"),
        read,
        PEEK_TIMEOUT,
    )
}

/// posts()'s loop: bucket k on, one peek each
fn posts(
    w: &World,
    i: Rc<Inst>,
    her: Ship,
    board: String,
    next: f64,
    k: u64,
    mut out: Vec<fu::Post>,
) -> Step {
    if ((k * 100) as f64) >= next {
        fu::sort_posts(&mut out);
        return Step::Ret(js(Js::Arr(out.iter().map(fu::Post::js).collect())));
    }
    let read = file_conv(|n| Ok(any(fu::posts(n)?)));
    let p = peek(
        w,
        &i,
        her,
        &format!("{board}/content/posts"),
        Some(&format!("b{k}")),
        read,
        PEEK_TIMEOUT,
    );
    wait_ok(Val::Promise(p), move |w, got| {
        // an empty bucket isn't there
        if let Some(got) = got.any::<Vec<fu::Post>>() {
            out.extend(got.iter().cloned());
        }
        posts(w, i, her, board, next, k + 1, out)
    })
}

/// +peer-base-cached: where the host runs lattice, for this reader's life
fn host_base(w: &World, i: &Rc<Inst>, st: &Rc<ForumSt>, host: &str) -> Promise {
    let (i, st, host) = (i.clone(), st.clone(), host.to_string());
    spawn(w, move |w| {
        if let Some(b) = st.base.get() {
            return Step::Ret(Val::Str(b.into()));
        }
        base_try(w, i, st, host, 0)
    })
}

fn base_try(w: &World, i: Rc<Inst>, st: Rc<ForumSt>, host: String, k: usize) -> Step {
    let Some(&b) = board::BASES.get(k) else {
        return throw(
            Failure::urb(format!(
                "{host} answered no lattice peek (offline, or no lattice)"
            ))
            .later(),
        );
    };
    let her = match ship_of(&host) {
        Ok(h) => h,
        Err(e) => return throw(e),
    };
    let read = Read {
        dir: false,
        conv: Conv2::Exists,
    };
    let p = peek(
        w,
        &i,
        her,
        &format!("{b}/pub"),
        Some("index"),
        read,
        board::BASE_WAIT,
    );
    wait_ok(Val::Promise(p), move |w, got| {
        if matches!(got, Val::Bool(true)) {
            st.base.set(Some(b));
            return Step::Ret(Val::Str(b.into()));
        }
        base_try(w, i, st, host, k + 1)
    })
}

/// board(name): the board, as a [`board::Board`]
fn board_of(w: &World, i: &Rc<Inst>, st: &Rc<ForumSt>, host: String, name: String) -> Promise {
    let (i, st) = (i.clone(), st.clone());
    spawn(w, move |w| {
        let p = grubs(w, &i, &st, host.clone(), name.clone());
        wait_ok(Val::Promise(p), move |_, acc| {
            let Some(acc) = acc.any::<RefCell<board::Acc>>() else {
                return throw(Failure::urb(format!(
                    "{host} shares no board {name} with {}: unreachable, private or not there",
                    i.our.borrow()
                )));
            };
            match acc.borrow().assemble() {
                Ok(b) => Step::Ret(any(b)),
                Err(e) => throw(e.into()),
            }
        })
    })
}

/// the board's two directories read into one acc, or null if either is
/// not there. One at a time, as lattice peeks them
fn grubs(w: &World, i: &Rc<Inst>, st: &Rc<ForumSt>, host: String, name: String) -> Promise {
    let (i, st) = (i.clone(), st.clone());
    spawn(w, move |w| {
        let hb = host_base(w, &i, &st, &host);
        wait_ok(Val::Promise(hb), move |w, b| {
            let Val::Str(b) = b else {
                return throw(Failure::error("no base"));
            };
            if !board::is_name(&name) {
                return throw(Failure::urb(format!("not a board name: {name}")));
            }
            let dir = format!("{b}/forum/boards/{name}");
            let acc = Rc::new(RefCell::new(board::Acc::new(&host)));
            grubs_try(w, i, host, dir, acc, 0)
        })
    })
}

fn grubs_try(
    w: &World,
    i: Rc<Inst>,
    host: String,
    dir: String,
    acc: Rc<RefCell<board::Acc>>,
    k: usize,
) -> Step {
    let Some(&sub) = ["pub", "content"].get(k) else {
        return Step::Ret(Val::Any(acc));
    };
    let her = match ship_of(&host) {
        Ok(h) => h,
        Err(e) => return throw(e),
    };
    let a = acc.clone();
    let read = Read {
        dir: true,
        conv: Conv2::Dir(Rc::new(move |files| {
            for (pax, n) in files {
                let mut at = vec![sub.to_string()];
                at.extend(pax);
                a.borrow_mut().read(&at, n)?;
            }
            Ok(Val::Bool(true))
        })),
    };
    let p = peek(
        w,
        &i,
        her,
        &format!("{dir}/{sub}"),
        None,
        read,
        PEEK_TIMEOUT,
    );
    wait_ok(Val::Promise(p), move |w, got| {
        if got.is_null() {
            return Step::Ret(Val::Null);
        }
        grubs_try(w, i, host, dir, acc, k + 1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the parent some_parent gives: None for none, Some(n) for [~ n]
    fn parent_of(p: Option<f64>) -> Option<crate::Int> {
        match some_parent(p).unwrap() {
            Value::Null => None,
            Value::Array(v) => match v.as_slice() {
                [Value::Null, Value::Int(n)] => Some(n.clone()),
                _ => panic!("not [~ n]"),
            },
            _ => panic!("not a unit"),
        }
    }

    #[test]
    fn a_parent_of_0_is_the_first_comment_not_none() {
        let int = |n: f64| js::bigint_of_number(n).unwrap();
        assert_eq!(parent_of(None), None);
        assert_eq!(parent_of(Some(0.0)), Some(int(0.0)));
        assert_eq!(parent_of(Some(1.0)), Some(int(1.0)));
        // as BigInt(NaN) is a RangeError
        assert!(some_parent(Some(f64::NAN)).is_err());
    }
}
