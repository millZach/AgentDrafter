import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { takeToken, wanted } from './api.ts';
import { App } from './App.tsx';
import init from './wasm/agentdrafter_viewer.js';
import './styles.css';

// The token is taken off the address first, before anything else runs.
const token = takeToken(window.location, window.sessionStorage, window.history);
const { source, sheet } = wanted(window.location.search);
const root = createRoot(document.getElementById('app') as HTMLElement);

init().then(
  () =>
    root.render(
      <StrictMode>
        <App token={token} source={source} sheet={sheet} />
      </StrictMode>,
    ),
  (e: unknown) =>
    root.render(
      <div className="va-note failed" role="alert">
        The viewer&rsquo;s renderer could not start: {String((e as Error)?.message ?? e)}
      </div>,
    ),
);
