# Parallel transfers

Independent copy/move batches start immediately by default. No manual parallel
mode or fixed one-job limit is used. Files inside each batch retain their ordered,
audited execution. A paused batch retains its path reservations.

Each running batch has a stable id, its own CopyControl, timer, progress,
phase, pause and cancel buttons. Completing/cancelling one does not reset another.
The bottom status bar presents aggregate progress and explicitly controls all jobs.
The transfer card scrolls when there are many batches; errors remain readable.

Read/read sharing is allowed. Moves reserve source trees for writing; copies
reserve sources for reading. Destinations reserve their item trees for writing.
Numbered copies reserve the destination directory while allocating names.
Overlapping jobs queue in order; unrelated jobs bypass a blocked queue head.
An unresolved conflict dialog waits for a decision while already-running jobs continue.

The journal session holds the existing OS executor lock across all parallel jobs.
Other windows/processes, Undo, recovery and exclusive mutations cannot enter that
gate. Within a session the core also rejects overlapping plan footprints before
writing an intent or changing a file. SQLite connections/transactions are short
lived and are not held over filesystem I/O. The lease is released after the last
batch finishes. Rename/delete/Undo keep their existing exclusive execution.

Automated regressions include a real paused audited copy alongside an independent
move, local cancellation, shared read leases, overlapping writer refusal, OS gate
retention, independent UI controllers and bypass of a blocked queue head.

Manual Windows acceptance:

1. Start three independent large copy/move batches: all should appear immediately.
2. Pause and cancel just one card: the others must continue with their own progress.
3. Finish the first batch while another is paused: remaining controls must still work.
4. Submit a move of a source currently being copied, or a second write into the same
   destination tree: it must wait, then recheck conflicts after the first finishes.
5. Queue a blocked batch followed by an unrelated one: the latter must start.
6. Check Ctrl+Z after all batches finish, retry after a failed network transfer,
   numbered names, replacement dialogs, staging restoration and another Papka window.

Mapped drive names are expanded where Windows supplies their universal names.
This scheduler coordinates Papka jobs; external programs do not participate in
its reservations, so the existing per-file revalidation and publication guards remain.
