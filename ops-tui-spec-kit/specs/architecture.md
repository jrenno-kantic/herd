# Architecture

## Layers

1. UI (ratatui)
2. App State
3. Services
4. Execution Engine

## Pattern
Component-based + Event-driven

## Data Flow
User Input → Event → Action → State Update → Render

## Key Modules

- app.rs → state
- tui.rs → rendering
- services → logic
- components → UI