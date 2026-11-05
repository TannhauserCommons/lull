# ADR-0040: Confirmations are a kind of toast: they collapse, and a listener can quiet them

- Status: accepted
- Date: 2026-11-05

## Context

Every note lull shows goes through one helper, `App.toast`. Most of those notes only confirm what the listener just
did ("Added to queue: X", "Added to Liked songs"). Queue a few songs and three of them stack up over the page; on a
phone they cover the player. Some actions even raised two notes for one press (an album's "Add to queue" in a menu,
then the player's own). Other notes matter: an error, a song skipped because no peer shares it, another device taking
over the music, an update ready. Hiding those would hide what is wrong or what lull is doing.

## Decision

**The code that raises a note says whether it is a confirmation.** `App.toast(message, { confirms: '<group>' })` marks
a note that only says an action did what the listener asked: queued, liked, followed, saved, added to a playlist,
copied, downloaded, removed, a switch flipped. Everything else is an alert: errors (never a confirmation, whatever
the call says), status (radio starting, a source switched, a song skipped), notes that say why nothing happened
("Already in your library", "This playlist is empty"), results the listener waits for (an import's counts, a found
source) and prompts (install, update). A note that says nothing is an alert, so a new one is never hidden by mistake.

**Confirmations of a group collapse.** One raised while another of its group is on screen updates that note instead
of stacking: its words count up (`n`, the things this one did, and `together(total)`, the words for them all: "Added
3 songs to the queue"), its timer starts again, and its Undo undoes every one of them, newest first. The groups are
what reads as one thing: the queue's additions, likes, unlikes, one playlist's additions, removals from the queue or
this device, downloads. A note's action names one function: the player raises the queue's confirmation, so a menu
that queues an album no longer raises a second one.

**A listener can quiet them.** Settings › Appearance › Confirmations: Show (the default, so nothing changes unasked)
or Quiet, per device like the other display settings (`lull:prefs`, `confirmations`). Quiet hides confirmations only.

**Screen readers hear them either way.** A quieted confirmation is still said, politely, through the page's announcer
(`#announcer`), and a collapsed one is said again as its words change (the toast region is `aria-live="polite"`).
What was done is never silent for someone who can't see it.

**Sounds are their own setting.** The UI sounds (ADR-0081) for liked, queued and saved keep their own switches in
Settings › Sounds, and Quiet doesn't touch them: a soft sound is the lighter confirmation a listener who quiets the
notes may want to keep, and one who doesn't turns that sound off where every other sound is.

## Alternatives

- **Classify by the toast's `kind`** (`ok`, `info`). Close at hand, but `kind` is how a note looks, and many
  confirmations have none while some status notes are `ok`; the meaning would follow the colour.
- **A list of confirmation messages in one place.** One file to read, but the words change and a list of strings
  drifts from them; the code that raises a note knows what it is.
- **Quiet by default.** Fewer notes for everyone at once, but it takes Undo off the screen unasked.
- **Only collapse, no setting.** Ends the stacking, but some listeners want no confirmations at all.

## Consequences

- **A new toast decides what it is:** pass `confirms` for a confirmation (and `n` and `together` when it collapses
  with others of its group); leave it out for anything the listener must see.
- **With Quiet, an Undo is gone with its note.** What was done is undone where it is shown (the heart, the queue's
  remove); Settings says what Quiet hides.
- **Themes need nothing new:** a collapsed note is the same `.toast`, its words changing in `.toast__msg`.
