// Desktop shell UI components for Issue #158 (Build 10 of 10):
// - StartScreen: AutoCAD-style dark welcome screen with Open folder/DWG/sheet
//   cards and persisted Recent drawings & sets (Zach's A1 & A2, 2026-11-22).
// - SwitcherModal: Title-bar switcher & Open… modal with Open / New window /
//   Remove on recents and Go to Start screen (Zach's A2 & A3).
// - DwgBanner: Plant DWG read-only / view-only (ADR-0004) / allow-loss / single
//   sheet info banner with Plot PDF action (Zach's A1 & A4).
// - DecodeReportModal: ADR-0004 decode report showing undecoded record kinds,
//   counts, hex handles, skipped/recovered/damage details, and explicit
//   checkbox acknowledgement to allow revising a drawing with loss.

import { useEffect, useState } from 'react';

import {
  kindBadge,
  nextRevisionHint,
  sourceDisplayName,
  undecodedSummary,
  type EditGuard,
  type ReadReport,
  type RecentEntry,
  type SourceKind,
} from './desktop.ts';

export interface StartScreenProps {
  recents: readonly RecentEntry[];
  error: string | null;
  busy: boolean;
  onDismissError(): void;
  onPickSet(): void;
  onPickDwg(): void;
  onPickSheet(): void;
  onOpenRecent(entry: RecentEntry, newWindow: boolean): void;
  onRemoveRecent(path: string): void;
}

/** AutoCAD-inspired dark Start screen shown when launched with no drawing open. */
export function StartScreen({
  recents,
  error,
  busy,
  onDismissError,
  onPickSet,
  onPickDwg,
  onPickSheet,
  onOpenRecent,
  onRemoveRecent,
}: StartScreenProps) {
  return (
    <div className="dk-shell" data-desktop-start>
      <header className="titlebar dk-startbar">
        <svg className="mark" viewBox="0 0 20 20" aria-hidden="true">
          <path d="M3 17V4l13 13z" fill="none" stroke="#4aa3ff" strokeWidth="2" strokeLinejoin="round" />
          <path d="M7 14v-3l3 3z" fill="#4aa3ff" />
        </svg>
        <span className="appname">AgentDrafter</span>
        <button
          type="button"
          className="qa dk-openbtn"
          data-act="desktop-open"
          disabled={busy}
          title="Open a drawing set folder (Ctrl+O)"
          onClick={onPickSet}
        >
          Open&#8230;
        </button>
        <div className="dk-switcher">
          <span className="dk-starttitle">Start — Pick a drawing set or file</span>
        </div>
      </header>
      <main className="dk-startview" aria-label="Start">
        <div className="dk-startinner">
          <div className="dk-hero">
            <div>
              <h1>
                <svg width="24" height="24" viewBox="0 0 20 20" aria-hidden="true">
                  <path d="M3 17V4l13 13z" fill="none" stroke="#4aa3ff" strokeWidth="2" strokeLinejoin="round" />
                  <path d="M7 14v-3l3 3z" fill="#4aa3ff" />
                </svg>
                AgentDrafter Desktop
              </h1>
              <p>
                Runs the Rust engine locally inside the app — no terminal or separate server needed. Open a drawing set folder, a
                plant&rsquo;s DWG original, or a single sheet description.
              </p>
            </div>
            <span className="dk-enginepill">
              <i aria-hidden="true" />
              Core ready &middot; local in-process
            </span>
          </div>

          {error && (
            <div className="dk-errbanner" role="alert" data-desktop-error>
              <span>{error}</span>
              <button type="button" className="btn sm" onClick={onDismissError}>
                Dismiss
              </button>
            </div>
          )}

          <div className="dk-opengrid">
            <button
              type="button"
              className="dk-opencard primarycard"
              data-pick="set"
              disabled={busy}
              onClick={onPickSet}
            >
              <span className="ochead">
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path d="M3 6h6l2 2h10v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
                </svg>
                Open drawing set folder&#8230;
              </span>
              <p>Pick a folder holding set.yaml, templates and rows. Opens every sheet live with journal, undo, accept and assistant.</p>
              <span className="ochint">Folder picker &middot; set.yaml</span>
            </button>

            <button type="button" className="dk-opencard" data-pick="dwg" disabled={busy} onClick={onPickDwg}>
              <span className="ochead">
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path d="M6 3h8l4 4v14H6z" />
                  <path d="M14 3v4h4M9 13h6M9 17h4" />
                </svg>
                Open plant DWG&#8230;
              </span>
              <p>
                Inspect, query, check and revise an existing .dwg. Original stays read-only; revisions save beside it in its exact DWG
                version.
              </p>
              <span className="ochint">File picker &middot; *.dwg (AC1012&ndash;AC1032)</span>
            </button>

            <button type="button" className="dk-opencard" data-pick="sheet" disabled={busy} onClick={onPickSheet}>
              <span className="ochead">
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <rect x="4" y="4" width="16" height="16" rx="1" />
                  <path d="M8 9h8M8 13h8M8 17h5" />
                </svg>
                Open single sheet description&#8230;
              </span>
              <p>Open one standalone .yaml sheet to view, measure, inspect objects and plot PDF without creating a set.</p>
              <span className="ochint">File picker &middot; *.yaml</span>
            </button>
          </div>

          <section aria-label="Recent drawings and sets">
            <div className="dk-secrow">
              <h2>Recent drawings &amp; sets</h2>
              <span className="dk-winhint">Opens here by default, or use New window to compare side by side</span>
            </div>
            {recents.length === 0 ? (
              <div className="dk-empty" data-recents-empty>
                No recent drawings yet. Open a drawing set folder, a plant DWG, or a YAML sheet above to get started.
              </div>
            ) : (
              <div className="dk-recentlist" data-recents-list>
                {recents.map((item) => (
                  <div key={item.path} className="dk-rec" data-recent={item.path}>
                    <span className={`dk-kbadge ${item.kind}`}>{kindBadge(item.kind)}</span>
                    <button
                      type="button"
                      className="dk-recmain"
                      onClick={() => onOpenRecent(item, false)}
                    >
                      <span className="dk-rectop">
                        <b>{item.name}</b>
                      </span>
                      <span className="dk-recpath">{item.path}</span>
                      {item.subtitle && <span className="dk-recmeta">{item.subtitle}</span>}
                    </button>
                    <div className="dk-recacts">
                      <button
                        type="button"
                        className="btn sm primary"
                        data-recent-open
                        onClick={() => onOpenRecent(item, false)}
                      >
                        Open
                      </button>
                      <button
                        type="button"
                        className="btn sm"
                        data-recent-newwin
                        onClick={() => onOpenRecent(item, true)}
                      >
                        New window
                      </button>
                      <button
                        type="button"
                        className="btn sm dk-rmbtn"
                        data-recent-remove
                        aria-label={`Remove ${item.name} from recent list`}
                        title="Remove from recents"
                        onClick={() => onRemoveRecent(item.path)}
                      >
                        &times;
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </section>
        </div>
      </main>
    </div>
  );
}

export interface SwitcherModalProps {
  currentPath: string;
  recents: readonly RecentEntry[];
  error: string | null;
  busy: boolean;
  onClose(): void;
  onGoStart(): void;
  onPickSet(newWindow: boolean): void;
  onPickFile(filter: 'dwg' | 'yaml' | undefined, newWindow: boolean): void;
  onOpenRecent(entry: RecentEntry, newWindow: boolean): void;
  onRemoveRecent(path: string): void;
}

/** Modal opened from the title-bar switcher or Open… (Ctrl+O) to switch drawings or open a new window. */
export function SwitcherModal({
  currentPath,
  recents,
  error,
  busy,
  onClose,
  onGoStart,
  onPickSet,
  onPickFile,
  onOpenRecent,
  onRemoveRecent,
}: SwitcherModalProps) {
  return (
    <div
      className="va-modal"
      role="dialog"
      aria-modal="true"
      aria-label="Switch drawing or open another"
      data-desktop-switcher
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="dialog dk-dialog">
        <header className="dk-dhead">
          <h3>Switch drawing or open another</h3>
          <button type="button" className="dk-dx" aria-label="Close" onClick={onClose}>
            &times;
          </button>
        </header>
        {error && (
          <div className="dk-errbanner" role="alert" data-desktop-error>
            <span>{error}</span>
          </div>
        )}
        <div className="dk-swacts">
          <button type="button" className="btn primary" data-sw-pick="set" disabled={busy} onClick={() => onPickSet(false)}>
            Open drawing set folder&#8230;
          </button>
          <button type="button" className="btn" data-sw-pick="file" disabled={busy} onClick={() => onPickFile(undefined, false)}>
            Open DWG or YAML file&#8230;
          </button>
          <button
            type="button"
            className="btn"
            data-sw-pick="set-newwin"
            disabled={busy}
            title="Pick a drawing set folder to open in a second window (Ctrl+Shift+O)"
            onClick={() => onPickSet(true)}
          >
            New window (folder)&#8230;
          </button>
          <button
            type="button"
            className="btn"
            data-sw-pick="file-newwin"
            disabled={busy}
            title="Pick a plant DWG or single YAML sheet to open in a second window"
            onClick={() => onPickFile(undefined, true)}
          >
            New window (DWG/YAML)&#8230;
          </button>
        </div>
        {recents.length > 0 ? (
          <div className="dk-recentlist dk-swrecents" data-switcher-recents>
            {recents.map((item) => {
              const isOpen = item.path === currentPath;
              return (
                <div key={item.path} className="dk-rec" data-recent={item.path}>
                  <span className={`dk-kbadge ${item.kind}`}>{kindBadge(item.kind)}</span>
                  <button type="button" className="dk-recmain" onClick={() => onOpenRecent(item, false)}>
                    <span className="dk-rectop">
                      <b>{item.name}</b>
                      {isOpen && <span className="pill dk-pill-ok">Open</span>}
                    </span>
                    <span className="dk-recpath">{item.path}</span>
                    {item.subtitle && <span className="dk-recmeta">{item.subtitle}</span>}
                  </button>
                  <div className="dk-recacts">
                    <button
                      type="button"
                      className="btn sm primary"
                      data-recent-open
                      onClick={() => onOpenRecent(item, false)}
                    >
                      {isOpen ? 'Focus' : 'Open'}
                    </button>
                    <button
                      type="button"
                      className="btn sm"
                      data-recent-newwin
                      onClick={() => onOpenRecent(item, true)}
                    >
                      New window
                    </button>
                    <button
                      type="button"
                      className="btn sm dk-rmbtn"
                      data-recent-remove
                      aria-label={`Remove ${item.name} from recent list`}
                      onClick={() => onRemoveRecent(item.path)}
                    >
                      &times;
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        ) : (
          <p className="muted">No other recent drawings yet.</p>
        )}
        <div className="dialog-acts">
          <button type="button" className="btn" data-sw-start onClick={onGoStart}>
            Go to Start screen
          </button>
          <button type="button" className="btn" data-cancel onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

export interface DwgBannerProps {
  kind: SourceKind;
  source: string;
  guard: EditGuard;
  readReport: ReadReport | null;
  readError: string | null;
  latestRevisionFile: string | null;
  plotting: boolean;
  onInspectDecode(): void;
  onPlotPdf(): void;
}

/**
 * Safety & status banner under the tabs for plant DWGs (ADR-0004) and single
 * YAML sheet descriptions. Hidden for drawing sets.
 */
export function DwgBanner({
  kind,
  source,
  guard,
  readReport,
  readError,
  latestRevisionFile,
  plotting,
  onInspectDecode,
  onPlotPdf,
}: DwgBannerProps) {
  if (kind === 'set') return null;

  const fileName = sourceDisplayName(source);

  if (kind === 'sheet') {
    return (
      <div className="dk-banner sheetinfo" role="status" data-dwg-banner="sheet">
        <span className="bmsg">
          <b>Single sheet description:</b> <code>{fileName}</code> &mdash; pan, zoom, inspect properties and plot PDF.
        </span>
        <span className="bacts">
          <button type="button" className="btn sm" data-act="plot-pdf" disabled={plotting} onClick={onPlotPdf}>
            {plotting ? 'Plotting…' : 'Plot PDF'}
          </button>
        </span>
      </div>
    );
  }

  // Plant DWG original (#70 / #71 / ADR-0004)
  if (guard === 'loading') {
    return (
      <div className="dk-banner clean" role="status" data-dwg-banner="loading">
        <span className="bmsg">
          <b>Inspecting original DWG:</b> reading decode report for <code>{fileName}</code> (view-only until verified)&hellip;
        </span>
      </div>
    );
  }

  if (readError || !readReport) {
    return (
      <div className="dk-banner loss" role="alert" data-dwg-banner="error">
        <span className="bmsg">
          <b>View-only (ADR-0004):</b> could not inspect decode report for <code>{fileName}</code>
          {readError ? ` (${readError})` : ''}. Editing is locked.
        </span>
      </div>
    );
  }

  const ver = readReport.dwg_version;
  const revName = latestRevisionFile ?? nextRevisionHint(fileName);

  if (guard === 'view-only') {
    return (
      <div className="dk-banner loss" role="alert" data-dwg-banner="view-only">
        <span className="bmsg">
          <b>View-only (ADR-0004):</b> <code>{fileName}</code> ({ver}) has {undecodedSummary(readReport)}. Revising is locked until
          you inspect and allow loss for this drawing.
        </span>
        <span className="bacts">
          <button type="button" className="btn sm dk-warnbtn" data-act="inspect-loss" onClick={onInspectDecode}>
            Inspect &amp; Allow revising&#8230;
          </button>
          <button type="button" className="btn sm" data-act="plot-pdf" disabled={plotting} onClick={onPlotPdf}>
            {plotting ? 'Plotting…' : 'Plot PDF'}
          </button>
        </span>
      </div>
    );
  }

  if (guard === 'allowed-loss') {
    return (
      <div className="dk-banner allowed" role="status" data-dwg-banner="allowed-loss">
        <span className="bmsg">
          <b>Allow-loss enabled for this drawing:</b> <code>{fileName}</code> stays untouched; revisions save beside it as{' '}
          <code>{revName}</code> in <code>{ver}</code>.
        </span>
        <span className="bacts">
          <button type="button" className="btn sm" data-act="inspect-loss" onClick={onInspectDecode}>
            Decode report
          </button>
          <button type="button" className="btn sm" data-act="plot-pdf" disabled={plotting} onClick={onPlotPdf}>
            {plotting ? 'Plotting…' : 'Plot PDF'}
          </button>
        </span>
      </div>
    );
  }

  return (
    <div className="dk-banner clean" role="status" data-dwg-banner="clean">
      <span className="bmsg">
        <b>Original DWG ({ver}):</b> <code>{fileName}</code> is read-only (decoded whole). Revisions save beside it as{' '}
        <code>{revName}</code> with full journal, undo &amp; accept.
      </span>
      <span className="bacts">
        <button type="button" className="btn sm" data-act="plot-pdf" disabled={plotting} onClick={onPlotPdf}>
          {plotting ? 'Plotting…' : 'Plot PDF'}
        </button>
      </span>
    </div>
  );
}

export interface DecodeReportModalProps {
  source: string;
  report: ReadReport;
  alreadyAllowed: boolean;
  onClose(): void;
  onAllowLoss(): void;
}

/**
 * ADR-0004 decode-report modal listing undecoded record kinds, counts, hex
 * handles, skipped/recovered/damage flags, and requiring explicit checkbox
 * acknowledgement before enabling `allow_loss: true` for this drawing/session.
 */
export function DecodeReportModal({ source, report, alreadyAllowed, onClose, onAllowLoss }: DecodeReportModalProps) {
  const [checked, setChecked] = useState(alreadyAllowed);
  useEffect(() => {
    setChecked(alreadyAllowed);
  }, [alreadyAllowed, source]);

  const fileName = sourceDisplayName(source);
  const revHint = nextRevisionHint(fileName);

  return (
    <div
      className="va-modal"
      role="dialog"
      aria-modal="true"
      aria-label={`Undecoded records in ${fileName}`}
      data-decode-modal
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="dialog dk-dialog warn">
        <header className="dk-dhead">
          <h3>
            Undecoded records in {fileName} ({report.dwg_version})
          </h3>
          <button type="button" className="dk-dx" aria-label="Close" onClick={onClose}>
            &times;
          </button>
        </header>
        <p>
          The DWG library could not decode every record in this original. Revising it may drop or leave out undecoded or damaged
          records listed below. <b>The original file is never overwritten</b> — any revision saves beside it as{' '}
          <code>{revHint}</code> in <code>{report.dwg_version}</code>.
        </p>

        {(report.skipped > 0 || report.recovered_errors > 0 || report.diagnostics_truncated || report.read_past_damage) && (
          <div className="dk-damagenote" data-decode-flags>
            {report.skipped > 0 && (
              <div>
                <b>Skipped records:</b> {report.skipped} (could not be read and would be left out)
              </div>
            )}
            {report.recovered_errors > 0 && (
              <div>
                <b>Recovered errors:</b> {report.recovered_errors}
              </div>
            )}
            {report.diagnostics_truncated && (
              <div>
                <b>Diagnostics truncated:</b> reader hit its diagnostic cap; additional undecoded records may exist.
              </div>
            )}
            {report.read_past_damage && (
              <div>
                <b>Read past damage:</b> {report.read_past_damage}
              </div>
            )}
          </div>
        )}

        {report.not_decoded.length > 0 && (
          <table className="dk-dtable" data-not-decoded-table>
            <thead>
              <tr>
                <th>Kind</th>
                <th>Count</th>
                <th>Handles (hex)</th>
                <th>Status</th>
              </tr>
            </thead>
            <tbody>
              {report.not_decoded.map((rec) => (
                <tr key={rec.kind}>
                  <td>
                    <code>{rec.kind}</code>
                  </td>
                  <td>{rec.count}</td>
                  <td>
                    <code>{rec.handles.join(', ') || '—'}</code>
                  </td>
                  <td>Not decoded whole by DWG library</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}

        {report.not_read.length > 0 && (
          <div className="dk-notread">
            <small className="muted">
              Objects the core has no model for (preserved untouched in clean revisions, not drawn):{' '}
              {report.not_read.map((r) => `${r.count} ${r.kind}`).join(', ')}.
            </small>
          </div>
        )}

        <label className="dk-guardcheck">
          <input
            type="checkbox"
            data-allow-loss-check
            checked={checked}
            onChange={(e) => setChecked(e.currentTarget.checked)}
          />
          <span>
            Allow revising <b>{fileName}</b> with loss for this drawing (<code>allow_loss: true</code>). Keep{' '}
            <code>{fileName}</code> untouched and write revisions beside it.
          </span>
        </label>

        <div className="dialog-acts">
          <button type="button" className="btn" data-keep-viewonly onClick={onClose}>
            {alreadyAllowed ? 'Close' : 'Keep view-only'}
          </button>
          {!alreadyAllowed && (
            <button
              type="button"
              className="btn dk-warnbtn"
              data-allow-loss-confirm
              disabled={!checked}
              onClick={onAllowLoss}
            >
              Allow revising this drawing
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
