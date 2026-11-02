:::  grubbery's molds for what urb-fetch sends to grubbery and reads
:::  from it, for check.mjs to clam every message mesa.mjs builds. Each
:::  block below a ::: line is copied verbatim from grubbery b126c50
:::  (desk/lib/tarball.hoon and desk/lib/nexus.hoon, which are the same
:::  at eed73a6), at the lines it names; molds.test.mjs checks that with
:::  a grubbery checkout. Only the molds the messages use are here.
:::
:::  check.mjs pins them as two cores, tarball then nexus, so nexus's
:::  rail:tarball and the rest resolve as they do in grubbery.
:::
=/  tarball
::: lib/tarball.hoon 4
|%
::: lib/tarball.hoon 5-6
+$  neck      rail                :: a nexus identity (directory-level mark)
+$  blot      rail                :: a mark identity (hierarchical)
::: lib/tarball.hoon 12
+$  bask      (pair blot noun)    :: grubbery page: blot-typed noun
::: lib/tarball.hoon 39-43
+$  rail  [=path name=@ta]        :: path to file (dir + filename)
+$  fold  path                    :: path to directory
+$  lane  (each rail fold)        :: [%& rail] file or [%| fold] directory
+$  bend  (pair @ud lane)         :: relative: steps up + destination lane
+$  road  (each lane bend)        :: [%& lane] absolute or [%| bend] relative
::: lib/tarball.hoon 59-63
+$  weir
  $:  make=(set road)  :: allowed destinations for %make, %cull, %sand
      poke=(set road)  :: allowed destinations for %poke
      peek=(set road)  :: allowed destinations for %peek
  ==
::: lib/tarball.hoon 72-78
+$  pulp
  $:  neck=(unit neck)
      weir=(unit weir)
      gain=?
      contents=(map @ta [=bask gain=?])
  ==
+$  bole  (axal pulp)
::: lib/tarball.hoon (the core's end)
--
=/  nexus
::: lib/nexus.hoon 3 (the core's start)
|%
::: lib/nexus.hoon 69
+$  weir  weir:tarball
::: lib/nexus.hoon 83
+$  wave  (axal [fold=cass:clay file=(map @ta cass:clay)])
::: lib/nexus.hoon 102
+$  case  $%([%ud p=@ud] [%da p=@da])
::: lib/nexus.hoon 439-443
+$  pace
  $%  [%firm p=(unit jobe)]
      [%temp p=(unit jobe)]
      [%tomb ~]
  ==
::: lib/nexus.hoon 500-528
+$  jobe  lobe:clay                ::  hash naming a ject (jects.silo)
+$  nobe  lobe:clay                ::  hash naming a noun (nouns.silo)
+$  leaf
  $:  lobe=nobe
      mark=[=blot:tarball ckey=@uv ns=path]
      gain=?
      bang=(unit nobe)
  ==
+$  tree
  $:  nek=(unit [=neck:tarball ckey=@uv ns=path])
      gain=?
      bang=(unit nobe)
      fil=(map @ta jobe)
      dir=(map @ta [lobe=jobe weir=(unit weir)])
  ==
+$  ject
  $%  [%leaf =leaf]
      [%tree =tree]
  ==
+$  vale  (map [nobe @uv] (unit tang))
::  kind-separated lobe sets: never flatten jects and nouns into one
::  bag — every silo operation is kind-directed, so consumers must
::  know which store a lobe lives in.
::
+$  lobes  [jects=(set jobe) nouns=(set nobe)]
+$  silo
  $:  nouns=(map nobe [refs=@ud =noun])
      jects=(map jobe [refs=@ud =ject])
  ==
::: lib/nexus.hoon 1296-1344
++  remo
  =<  remo
  |%
  +$  remo  [=peeks =snaps]
  +$  load
    $:  [=wire dest=lane:tarball]
        $%  [%make force=? gain=? =make]
            [%cull ~]
            [%sand weir=(unit weir)]
            [%load ~]
            [%poke =bask:tarball]
            [%peek case=(unit case) deep=?]
            [%keep ~]
            [%drop ~]
        ==
    ==
  ::  Runtime-to-runtime content-addressed negotiation.
  ::  Separate from load (dart-like ops) and intake (subscription events).
  ::
  +$  transfer
    $:  =wire
        ::  %snap's data field carries the content inline when the
        ::  server judged it cheaper to send than to negotiate — the
        ::  receiver merges it on arrival and skips the want/data
        ::  legs. Wire-only: inline data is never stored in a peek.
        $%  [%snap dest=lane:tarball snap-id=@uvJ snap=(unit snap) data=(unit silo)]
            [%veto dest=lane:tarball]
            [%want dest=lane:tarball snap-id=@uvJ]
            [%data =silo]
            [%miss ~]
            ::  consumption result of a %poke load the sender ran for
            ::  us; wire is the sender-encoded return address from
            ::  +dart-poke's remote branch. The cross-ship form of a %pack.
            [%pack err=(unit tang)]
        ==
    ==
  +$  make  (each bole:tarball [=bask:tarball blot=(unit blot:tarball)])
  ::  Inbound subscription events from remote watchers.
  ::
  +$  intake  [=wire dest=lane:tarball =wave]
  ::  Staged peek state: tracks an outstanding cross-ship peek
  ::  from initiation through negotiation to discharge.
  ::  snap is ~ until %snap response arrives with pace + refs.
  ::
  +$  snap   [=cass:clay =pace refs=lobes]
  +$  peek   [ship=@p dest=lane:tarball deep=? blot=(unit blot:tarball) snap=(unit snap) snap-id=@uvJ]
  +$  peeks  (map [rail:tarball wire] peek)
  +$  snaps  (map [@uvJ @p] [dest=lane:tarball refs=lobes expiry=@da])
  --
::: lib/nexus.hoon (the core's end)
--
