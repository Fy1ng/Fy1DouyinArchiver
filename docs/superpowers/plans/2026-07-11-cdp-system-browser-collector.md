# CDP System Browser Collector Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace Playwright with direct CDP control of system Edge and Chrome while preserving collector schema v2 and the React snapshot state machine.

**Architecture:** Use `chrome-remote-interface` behind a small `CdpPage` adapter. Browser discovery, profile selection, login validation, process lifecycle, and CDP transport remain isolated from snapshot and sync-engine logic.

**Tech Stack:** TypeScript, Node.js 24, chrome-remote-interface 0.34.0, Vitest, Rust/Tauri 2, @vercel/ncc.

## Global Constraints

- Windows x64 only; support system Edge and Chrome in that priority order.
- Edge and Chrome use separate new profiles and never share login state.
- Do not read, migrate, or delete the old Playwright profile.
- Tauri always uses automatic browser selection; `--browser edge|chrome` is internal for development and acceptance.
- Keep CollectorSuccessV2, React snapshot behavior, sync state machine, schema v2, and existing history data unchanged.
- Remove Playwright from production, tests, source, build output, and NSIS resources.

---

### Task 1: Browser discovery, state, and CLI contracts

Add tested Edge/Chrome discovery, browser selection/login marker persistence, new browser-data and PID CLI parameters, and stable browser error descriptions.

### Task 2: CDP session and page adapter

Launch a dedicated system browser with a random loopback debugging port, read DevToolsActivePort, connect with chrome-remote-interface, provide navigate/evaluate/wait/resource-blocking operations, and close or terminate the browser process tree.

### Task 3: Collector integration

Migrate login and collection commands plus snapshot/readiness/scroll modules to CdpPage. Preserve collection semantics and add cooperative cancellation with browser PID reporting.

### Task 4: Tauri runtime integration

Pass app-data browser state and browser PID paths, remove legacy login-marker prechecks, map CDP errors to Chinese messages, and force-clean browser processes after cancellation when needed.

### Task 5: Test and package migration

Replace Playwright fixtures with a CDP fixture server, remove legacy Playwright POC/build logic/dependencies, update documentation, and run all automated and package gates.
