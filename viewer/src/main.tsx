import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { takeToken } from './api.ts';
import { App } from './App.tsx';
import { getDesktopBridge, parseDesktopUrl } from './desktop.ts';
import init from './wasm/agentdrafter_viewer.js';
import './styles.css';

// The token is taken off the address first, before anything else runs.
const token = takeToken(window.location, window.sessionStorage, window.history);
const initial = parseDesktopUrl(window.location.search, getDesktopBridge() !== null);
const root = createRoot(document.getElementById('app') as HTMLElement);

init().then(
  () =>
    root.render(
      <StrictMode>
        <App
          token={token}
          source={initial.source ?? '.'}
          kind={initial.kind}
          sheet={initial.sheet}
          layout={initial.layout}
          desktop={initial.desktop}
          start={initial.start}
        />
      </StrictMode>,
    ),
  (e: unknown) =>
    root.render(
      <div className="va-note failed" role="alert">
        The viewer&rsquo;s renderer could not start: {String((e as Error)?.message ?? e)}
      </div>,
    ),
);
