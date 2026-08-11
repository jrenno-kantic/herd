# Layout

Terminal geometry, built in `layout.rs`.

```
+--------------+-----------------------+
| Sidebar      | Active screen         |
| (24 cols)    |                       |
| 1 Models     | Models:   table       |
| 2 Server     |           + argv      |
| 3 Test       | Server:   summary     |
| 4 Stats      |           + log tail  |
| 5 Settings   | Test:     request     |
| 6 Logs       |           + response  |
|              | Stats:    session     |
| tier  32gb   |           + memory    |
| RAM   36 GiB | Settings: key list    |
|              | Logs:     history     |
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
  | Models | preset table (min 3) | argv preview (8) |
  | Server | state summary (10) | recent output (min 3) |
  | Test | request (8) | response (min 3) |
  | Stats | session counters (12) | memory budget (min 5) |

- Two modals are centred over the whole frame, each clamped so it still fits in
  a terminal smaller than the modal itself: the port-conflict confirmation and
  the `models.ini` picker
- The status bar leads with the lifecycle tag, coloured by state, then the model,
  endpoint, uptime and a hint for the current mode
