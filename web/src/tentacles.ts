import "./style.css";

import { initializeLeaderboard, type LeaderboardController } from "./leaderboard";

function requireElement<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) throw new Error(`Missing required Tentacle leaderboard element: ${id}`);
  return element as T;
}

let controller: LeaderboardController | undefined;

try {
  controller = initializeLeaderboard({
    root: requireElement<HTMLElement>("tentacles"),
    status: requireElement<HTMLElement>("leaderboard-state"),
    source: requireElement<HTMLElement>("leaderboard-source"),
    summary: requireElement<HTMLElement>("leaderboard-summary"),
    refresh: requireElement<HTMLButtonElement>("leaderboard-refresh"),
    search: requireElement<HTMLInputElement>("leaderboard-search"),
    funding: requireElement<HTMLSelectElement>("leaderboard-funding"),
    verification: requireElement<HTMLSelectElement>("leaderboard-verification"),
    protocol: requireElement<HTMLSelectElement>("leaderboard-protocol"),
    shared: requireElement<HTMLSelectElement>("leaderboard-shared"),
    sort: requireElement<HTMLSelectElement>("leaderboard-sort"),
    ranked: requireElement<HTMLElement>("leaderboard-list"),
    suspendedSection: requireElement<HTMLElement>("leaderboard-suspended-section"),
    suspended: requireElement<HTMLElement>("leaderboard-suspended"),
    empty: requireElement<HTMLElement>("leaderboard-empty"),
  });
} catch (error) {
  console.error("The public leaderboard configuration is invalid", error);
  requireElement<HTMLElement>("leaderboard-state").textContent = "UNAVAILABLE";
  requireElement<HTMLElement>("leaderboard-source").textContent =
    "The Tentacle index could not be initialized";
}

window.addEventListener("pagehide", (event) => {
  if (!event.persisted) controller?.dispose();
});
