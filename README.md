# tui-tui

Two people, two terminals, one game. No server, no account, no port forwarding.

You read out a code like `42-tiger-marble-ocean`, your friend types it in, and
you are playing. The code is not an address on somebody's server — it *is* the
address, stretched into a keypair and looked up on a public DHT. Play chess,
or race each other to the same Wordle; the lobby, pairing and friends are
shared, so more can follow.

![two players, one keyboard, and a fool's mate](demo.gif)

```
╭───────────────── tui-tui ──────────────────╮
│                                            │
│   Game        ‹ Chess ›                    │
│                                            │
│   Host a game                              │
│   get a code to send your opponent         │
│   Join a game                              │
│   type in the code your opponent sent      │
│   Play on one keyboard                     │
│   two players taking turns                 │
│                                            │
│ friends                                    │
│ ▸ alice              3 games · 3h ago      │
│   bob                1 game · yesterday    │
│                                            │
│   Your name                                │
│   ace — what friends see                   │
│   Quit                                     │
│                                            │
│ code 42-tiger-marble-ocean                 │
╰────────────────────────────────────────────╯
```

## Install

```
brew install mr-nitesh-poudel/tap/tuitui
```

No Homebrew? A prebuilt binary for macOS and Linux:

```
curl -LsSf https://github.com/mr-nitesh-poudel/tui-tui/releases/latest/download/tui-tui-installer.sh | sh
```

Or from a clone, with `cargo install --path .`.

## Play

```
tuitui                            # the lobby
tuitui host                       # or skip it: host a game,
tuitui join 42-tiger-marble-ocean # join one,
tuitui local                      # or share a keyboard
tuitui play wordle                # the lobby, on a game of your choosing
tuitui local wordle               # wordle on your own
```

Every command but `join` takes an optional game. Joining never needs one: you
get whatever the host is playing. In chess, the host plays white.

Codes are forgiving. Any case, spaces instead of dashes, and four letters a
word is plenty — `42 tige marb ocea` gets you there. Tab finishes a word, and
a word that isn't in the list is flagged while you type it. Paste the whole
`tuitui join ...` command if that's what landed in your clipboard.

## At the board

Click a piece and click where it goes, or drag it. Right-click puts it back
down. The keyboard does everything too:

| key | |
|---|---|
| arrows / `hjkl` | move the cursor |
| `enter` / `space` | pick up, put down |
| `f` | flip the board |
| `p` | cycle piece style |
| `m` | hand the mouse back to your terminal |
| `c` | copy your share code |
| `r` / `d` | resign / offer a draw |
| `t` | chat with your opponent (or click the chat panel); `enter` sends, `esc` goes back to the board |
| `a` | analyse with an engine: an evaluation bar, the best move, and each move graded (hot-seat, or once a game is over) |
| `,` / `.` | step back and forward through the game (the arrows too, once it is over); `esc` comes back |
| `q` / `esc` | leave (it asks first) |

Promotion opens a prompt: click, or `←`/`→` and `enter`, or just press
`q` `r` `b` `n`.

Checkmate is not a status line. The board goes dark, the square flashes red,
and the losing king topples over away from whatever mated it before the
verdict is spelled out across the board. Resigning lays your king down gently
instead. Any key puts the board back.

## Analysis

With [Stockfish](https://stockfishchess.org) installed (`brew install
stockfish`, `apt install stockfish`), `a` analyses the game: an evaluation bar
beside the board, the engine's best move lit up in blue, and every move
graded — `?!` an inaccuracy, `?` a mistake, `??` a blunder. Step through the
game with `,` and `.`, or the arrows once it's over. Any other UCI engine
works too: point `TUITUI_ENGINE` at it.

Installed with Homebrew, Stockfish comes with it. Otherwise, if `a` finds no
engine it offers to download Stockfish from its official releases (about
80 MB, once): the file is checked against a checksum pinned in tuitui before
it's unpacked, and kept in tuitui's data folder with its licence.

An engine is advice, so it isn't available during a game against someone
else, only once it's over. Hot-seat can use it any time.

## Wordle

Both players get the same five-letter word and six guesses to find it, racing
each other. You see the colours of your opponent's guesses as they land, but
not the letters, until you're done yourself. Fewer guesses wins, and if you
both take the same number, whoever got there sooner wins. Rounds keep going
with a running score until someone leaves. At one keyboard it's a game for
one, with your streak and how many guesses each word took.

Type to guess, or click the keyboard on screen. It colours each letter with
what you know about it, once that guess has turned over. The tiles and keys
grow with your terminal, up to big pixel letters on a large screen. When a
round is settled, a card shows who won and spells out the word; `enter` goes
again, and any other key puts the card away so you can look at both boards.

| key | |
|---|---|
| letters, `⌫`, `enter` | type a guess, and make it |
| `enter` | between rounds: ready for the next one (both players press it) |
| `h` | between rounds: hard mode, where letters you've found must be used |
| `tab` / `t` | chat (`tab` while guessing, when letters are taken) |
| `esc` | leave (it asks first while a round is on) |

Neither player picks the word. Each side sends a hash of a random secret
first and reveals the secret afterwards, and the word comes from both secrets.
Each side also checks every guess the other makes itself. The word lists come
from [SCOWL](http://wordlist.aspell.net/): about 2,000 common words as
answers, and about 9,000 more accepted as guesses.

## Friends

Anyone you play turns up in the lobby under **friends**, and challenging one
takes no code at all — their lobby just asks them to accept. Leaving a game
drops you back with your last opponent already selected, so a rematch is one
keypress. `x` forgets someone.

## Pieces

The board sizes itself to your terminal, from 3x1 squares up to 11x5, and
draws the best pieces the square can carry: vector silhouettes stamped dot by
dot with Unicode octants, half-block sprites below that, then character art,
figurines and plain letters. A sliding piece moves a dot at a time, not a cell
at a time. If your terminal can't draw octants you'll see `�` — press `p` once
for braille, which everything can draw.

## How it finds your friend

A code is a number and three words: about 40 bits, short enough to read out
and far too many to guess at one try per connection.

1. Both sides stretch the code with Argon2id into the same keypair. The host
   signs a record naming its address and publishes it to the Mainline DHT —
   BitTorrent's, millions of nodes, nobody's server. The joiner derives the
   same key and looks it up.
2. Both prove they hold the code with SPAKE2, bound to both endpoint ids. A
   stranger gets one guess per connection, and three wrong ones and the host
   stops listening.
3. The host names its game, the joiner accepts or backs out.

Codes expire after an hour, or the moment the host leaves.

There is no referee. Both sides run the same rules over their own copy of the
position, so nobody has to trust the other's arithmetic.

## Hacking on it

Each game lives in its own module under `games/` and implements `Play`: its
own rules, keys and screen. It sits at a `Table`, which handles everything
games share — leaving, the mouse, the share code, the connection — so a game
never touches the network or quitting itself. `hub.rs` runs the loop without
knowing which game it is running.

Adding a game is a module, a `Play` implementation and one line in
`Kind::ALL`. `games/mod.rs` is the contract; `tests/table.rs` proves it with a
toy game that isn't chess, and Wordle is a second real one.

Nothing from the opponent is trusted: lines are length-capped before
authentication, names are cleaned on arrival, and a game checks every message
against its own state. `unsafe` is forbidden, and release builds keep overflow
checks on.

## Tests

```
cargo test                  # rules, hit-testing, drawing, two real endpoints talking
cargo test -- --ignored     # pairing over the real DHT, needs the internet
cargo run --example render  # prints the UI at several sizes, for layout work
```

One test draws and clicks every screen across terminal sizes from 0x0 up to a
maximised window, because a screen too small for a board is where the
arithmetic gives out. Locally it samples the sizes to keep `cargo test` quick;
`TUITUI_FULL_SWEEP=1 cargo test` takes every one.

CI runs all of it — formatting, clippy, tests and docs on Linux and macOS, the
full size sweep, and a build on Rust 1.95, the oldest supported — on every
push and pull request.

## Licence

MIT or Apache-2.0, your pick.
