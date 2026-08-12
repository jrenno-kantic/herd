# Layout

Terminal geometry, built in `layout.rs`.

```
+--------------+-----------------------+
| Sidebar      | Active screen         |
| (24 cols)    |                       |
| 1 Models     | Models:   table       |
| 2 Server     |           + argv      |
| 3 Router     | Server:   summary     |
| 4 Test       |           + log tail  |
| 5 Stats      | Router:   settings    |
| 6 Settings   |           + argv      |
| 7 Logs       | Test:     request     |
| 8 Hub        |           + response  |
|              | Stats:    session     |
| tier  32gb   |           + memory    |
| RAM   36 GiB | Settings: key list    |
|              | Logs:     history     |
|              | Hub:      cache list  |
+--------------+-----------------------+
| Command Bar                (3 rows)  |
+--------------------------------------+
| Status Bar                 (1 row)   |
+--------------------------------------+
```

- Sidebar is a fixed 24 columns and also shows the active tier and installed RAM;
  the screen area takes the rest
- Every screen but Settings and Logs splits its area in two:

  | Screen | Top | Bottom |
  |---|---|---|
  | Models | preset table (min 3) | argv preview (8), or the download bar |
  | Server | state summary (10) | recent output (min 3) |
  | Router | settings (min 3) | argv preview (8) |
  | Test | request (8) | response (min 3) |
  | Stats | session counters (12) | memory budget (min 5) |

  The Hub screen is one pane: a column header, the cache list, then a summary
  line (models, disk, how many this tier does not name) and the key hints.

- The Models table **sizes itself to the width it is given**. It was a fixed 89
  columns, so on a 100-column terminal — 74 for that pane once the sidebar and
  borders are taken — the right-hand columns were clipped off the edge with
  nothing to say they existed. The repo column shrinks first, then columns are
  dropped in a stated order (ctx → opt → caps → spec → ram); the marker, name
  and `LOCAL` never are
- Its footer is **two lines**: what the highlighted preset is, then what the keys
  do. On one line the description pushed the key hints off the right edge
- The Hub list sizes itself the same way (disk usage goes before the preset
  name, since colour already says whether a row is unreferenced but cannot say
  *which* preset uses it), and elides a long repo reference from the **left**,
  because the tag at the end is what identifies it
- **A scrollbar** is drawn on the right border of the Models and Hub lists when
  there are more rows than fit, and only then. It spans the list rows rather
  than the whole pane, and its position reproduces the offset `List` will
  choose, so the thumb cannot disagree with the rows beside it
- Modals are centred over the whole frame, each clamped so it still fits in a
  terminal smaller than the modal itself: the launch confirmation (port in use /
  too large / not downloaded), the quit confirmation, the `models.ini` picker,
  the `?` key reference and the `:help` command listing. The listing sizes
  itself from its content and elides visibly when the terminal is narrower
- The status bar leads with the lifecycle tag, coloured by state, then the model,
  the phase within that state, the elapsed time, endpoint and a hint for the
  current mode
- The command bar carries a hint on its own border: what the typed line would
  run, or where the command listing is when nothing has been typed
