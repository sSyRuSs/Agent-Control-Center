import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AlertCircle, CheckCircle2, Database, LoaderCircle, RefreshCw, Save } from 'lucide-react';
import './model-settings.css';

type Model = { id: string; label: string; efforts: string[]; defaultEffort: string };
type Role = { key: string; label: string; configPath: string; model: string; effort: string; revision: string; editable: boolean; warning: string };
type Settings = { sourceDir: string; catalogSource: string; catalogFetchedAt: string; warning: string; models: Model[]; roles: Role[] };
type Draft = { model: string; effort: string };
type SaveResult = { settings: Settings; backupPath: string; changed: boolean };

const formatWhen = (value: string) => {
  if (!value) return 'time unavailable';
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString('vi-VN');
};

export function ModelSettings() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const [loading, setLoading] = useState(true);
  const [loadingError, setLoadingError] = useState('');
  const [saving, setSaving] = useState<string | null>(null);
  const [results, setResults] = useState<Record<string, { error?: string; backupPath?: string; changed?: boolean }>>({});

  const load = useCallback(async () => {
    setLoading(true);
    setLoadingError('');
    try {
      const next = await invoke<Settings>('get_model_settings');
      setSettings(next);
      setDrafts(Object.fromEntries(next.roles.map(role => [role.key, { model: role.model, effort: role.effort }])));
      setResults({});
    } catch (error) {
      setLoadingError(String(error));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  const modelFor = (id: string) => settings?.models.find(model => model.id === id);
  const updateDraft = (role: Role, update: Partial<Draft>) => {
    setDrafts(current => {
      const draft = { ...current[role.key], ...update };
      if (update.model) {
        const model = modelFor(update.model);
        if (model && !model.efforts.includes(draft.effort)) draft.effort = model.defaultEffort;
      }
      return { ...current, [role.key]: draft };
    });
    setResults(current => { const { [role.key]: _removed, ...rest } = current; return rest; });
  };

  const save = async (role: Role) => {
    const draft = drafts[role.key];
    if (!draft) return;
    setSaving(role.key);
    setResults(current => { const { [role.key]: _removed, ...rest } = current; return rest; });
    try {
      const response = await invoke<SaveResult>('save_agent_model', { roleKey: role.key, model: draft.model, effort: draft.effort, revision: role.revision });
      const updated = response.settings;
      setSettings(current => {
        if (!current) return updated;
        const savedRole = updated.roles.find(item => item.key === role.key);
        if (!savedRole) return current;
        return {
          ...current,
          models: updated.models,
          catalogSource: updated.catalogSource,
          catalogFetchedAt: updated.catalogFetchedAt,
          warning: updated.warning,
          roles: current.roles.map(item => item.key === role.key ? savedRole : item),
        };
      });
      const savedRole = updated.roles.find(item => item.key === role.key);
      if (savedRole) setDrafts(current => ({ ...current, [role.key]: { model: savedRole.model, effort: savedRole.effort } }));
      setResults(current => ({ ...current, [role.key]: { changed: response.changed, backupPath: response.backupPath } }));
    } catch (error) {
      setResults(current => ({ ...current, [role.key]: { error: String(error) } }));
    } finally {
      setSaving(null);
    }
  };

  const changedRoles = useMemo(() => settings?.roles.filter(role => {
    const draft = drafts[role.key];
    return draft && (draft.model !== role.model || draft.effort !== role.effort);
  }).length ?? 0, [drafts, settings]);

  if (loading) return <section className="model-settings-loading" aria-live="polite"><LoaderCircle size={22}/> Loading model settings…</section>;
  if (loadingError) return <section className="model-settings-loading model-settings-error" role="alert"><AlertCircle size={22}/><div><b>Could not load model settings</b><span>{loadingError}</span><button onClick={() => void load()}><RefreshCw size={15}/> Try again</button></div></section>;
  if (!settings) return null;

  const pendingSave = saving !== null;

  return <div className="model-settings">
    <div className="model-heading">
      <div>
        <div className="eyebrow"><span className="eyebrow-line"/> SOFTWARE TEAM DEFAULTS</div>
        <h1>Model settings<span className="h1-dot">.</span></h1>
        <p>Set global defaults for new Codex sessions in the Software Team.</p>
      </div>
      <button className="refresh-settings" onClick={() => void load()} disabled={pendingSave} aria-label="Reload model settings and discard unsaved drafts" title="Discard unsaved drafts and reload current settings"><RefreshCw size={16}/> Reload</button>
    </div>

    <div className="settings-notice"><AlertCircle size={17}/><div><b>Changes apply to new Codex sessions.</b><span>Project settings or an explicit task setting can override these global defaults. Existing running agents keep their current model.</span></div></div>
    <div className="catalog-meta"><Database size={15}/><span>Catalog: {settings.catalogSource || 'cached locally'} · updated {formatWhen(settings.catalogFetchedAt)}</span><small>Cached catalog only; it does not verify account access.</small></div>
    {settings.warning && <div className="settings-warning" role="status"><AlertCircle size={15}/>{settings.warning}</div>}
    {changedRoles > 0 && <div className="unsaved-banner" role="status">{changedRoles} unsaved {changedRoles === 1 ? 'change' : 'changes'} — select Save for each role to apply.</div>}

    {settings.models.length === 0 ? <section className="model-empty"><Database size={24}/><b>No cached model catalog is available.</b><span>Reload after Codex has refreshed its local model catalog.</span></section> : <section className="model-role-list" aria-label="Software Team model defaults">
      {settings.roles.map(role => {
        const draft = drafts[role.key] ?? { model: role.model, effort: role.effort };
        const selected = modelFor(draft.model);
        const effortSupported = Boolean(selected && draft.effort && selected.efforts.includes(draft.effort));
        const dirty = draft.model !== role.model || draft.effort !== role.effort;
        const result = results[role.key];
        return <article className="model-role-card" key={role.key}>
          <div className="role-card-title"><div><h2>{role.label}</h2><span>{role.configPath || 'Global Codex default'}</span></div><span className={dirty ? 'draft-pill dirty' : 'draft-pill'}>{dirty ? 'Unsaved' : 'Saved'}</span></div>
          {role.warning && <div className="role-warning"><AlertCircle size={14}/>{role.warning}</div>}
          <div className="model-fields">
            <label>Model<select value={draft.model} onChange={event => updateDraft(role, { model: event.target.value })} disabled={!role.editable || pendingSave} aria-label={`${role.label} model`}>
              <option value="">Inherited / not set</option>
              {!selected && draft.model && <option value={draft.model}>{draft.model} (not in cached catalog)</option>}
              {settings.models.map(model => <option key={model.id} value={model.id}>{model.label || model.id}</option>)}
            </select></label>
            <label>Reasoning effort<select value={draft.effort} onChange={event => updateDraft(role, { effort: event.target.value })} disabled={!role.editable || !selected || pendingSave} aria-label={`${role.label} reasoning effort`}>
              <option value="">Inherited / not set</option>
              {selected && draft.effort && !selected.efforts.includes(draft.effort) && <option value={draft.effort}>{draft.effort} (not supported by cached catalog)</option>}
              {!selected && draft.effort && <option value={draft.effort}>{draft.effort} (availability unknown)</option>}
              {selected?.efforts.map(effort => <option key={effort} value={effort}>{effort}</option>)}
            </select></label>
          </div>
          <div className="role-save-row"><button className="save-model" disabled={!role.editable || !dirty || !effortSupported || pendingSave} onClick={() => void save(role)}>{saving === role.key ? <LoaderCircle size={15}/> : <Save size={15}/>}{saving === role.key ? 'Saving…' : 'Save role'}</button>{!role.editable && <span>This role is not editable.</span>}{role.editable && dirty && !effortSupported && <span>Select a cached model and supported effort to save.</span>}</div>
          {result?.error && <div className="save-result error" role="alert"><AlertCircle size={14}/><span>{result.error}</span></div>}
          {result && !result.error && <div className="save-result success" role="status"><CheckCircle2 size={14}/><span>{result.changed ? 'Saved global default.' : 'Already saved.'}{result.backupPath ? ` Backup: ${result.backupPath}` : ''}</span></div>}
        </article>;
      })}
    </section>}
    <p className="settings-footnote">Saving updates global Codex defaults only. The Research Team remains monitor-only.</p>
  </div>;
}
