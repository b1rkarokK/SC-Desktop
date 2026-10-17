// «Играть на»: a window above the player bar (like the queue and the equalizer) that shows this computer and the
// phones that are linked (settings → Интеграции → «Подключить телефон»), where the music plays now, and lets it be
// handed over: «Играть здесь» takes what plays on a phone, «Играть там» sends what plays here to the phone.
import { listen } from '@tauri-apps/api/event';
import { api, errorMessage, type DevicesView, type DeviceInfo } from './api';
import { h, toast } from './dom';
import { I, icon, iconButton } from './icons';
import { T } from './i18n';
import { store } from './store';

export class DevicesPopover {
  readonly el: HTMLDivElement;
  private body = h('div', { class: 'q-body dv-body' });
  private open = false;
  private view: DevicesView | null = null;
  private onToggle?: (open: boolean) => void;
  private refreshTimer = 0;

  constructor() {
    const close = iconButton(I.close, T('Закрыть'));
    close.addEventListener('click', () => this.setOpen(false));
    this.el = h(
      'div',
      { class: 'q-pop panel dv-pop', role: 'dialog', 'aria-label': T('Играть на') },
      h('div', { class: 'q-head' }, h('span', { class: 'strong', text: T('Играть на') }), h('div', { class: 'spacer' }), close),
      this.body,
    );
    this.el.hidden = true;
    const later = () => {
      if (!this.open) return;
      window.clearTimeout(this.refreshTimer);
      this.refreshTimer = window.setTimeout(() => void this.refresh(), 150);
    };
    void listen('devices:changed', later);
    void listen('player:state', later);
    document.addEventListener('keydown', (e) => e.key === 'Escape' && this.open && this.setOpen(false));
  }

  init(onToggle: (open: boolean) => void): void {
    this.onToggle = onToggle;
  }

  toggle(): void {
    this.setOpen(!this.open);
  }

  setOpen(open: boolean): void {
    if (this.open === open) return;
    this.open = open;
    this.el.hidden = !open;
    this.onToggle?.(open);
    if (open) void this.refresh();
    else this.body.replaceChildren(); // nothing kept while closed
  }

  get isOpen(): boolean {
    return this.open;
  }

  private async refresh(): Promise<void> {
    try {
      this.view = await api.devicesView();
    } catch (e) {
      toast(errorMessage(e), 'error');
      return;
    }
    if (this.open) this.render(this.view);
  }

  private render(v: DevicesView): void {
    const here = store.snapshot;
    const playingHere = !!here?.playing;
    const parts: HTMLElement[] = [];
    parts.push(this.row({ name: v.name || T('Этот компьютер'), sub: here?.track ? `${here.track.artist} · ${here.track.title}` : T('ничего не играет'), kind: 'pc', active: playingHere || !v.peers.some((p) => this.plays(p)), onClick: null }));
    for (const p of v.peers) {
      const playing = this.plays(p);
      const sub = !p.online ? T('не в сети') : p.remote?.track ? `${playing ? '▶ ' : ''}${p.remote.track.artist} · ${p.remote.track.title}` : T('в сети');
      parts.push(
        this.row({
          name: p.name || T('Телефон'),
          sub,
          kind: p.kind,
          active: playing && !playingHere,
          enabled: p.online,
          // the music goes where it was asked: what plays here to the phone, or a phone's to here
          onClick: p.online ? () => void this.pick(p, playingHere) : null,
          volume: playing && !playingHere ? p : undefined,
        }),
      );
    }
    if (!v.linked) {
      parts.push(h('p', { class: 'muted small dv-hint', text: T('Телефон ещё не подключён: Настройки, Интеграции, «Подключить телефон».') }));
    } else if (!v.peers.length) {
      parts.push(h('p', { class: 'muted small dv-hint', text: T('Откройте SC Desk на телефоне: он появится здесь.') }));
    }
    this.body.replaceChildren(...parts);
  }

  private plays(p: DeviceInfo): boolean {
    const r = p.remote;
    return !!r?.playing && p.online && !!r.track;
  }

  private async pick(p: DeviceInfo, playingHere: boolean): Promise<void> {
    try {
      // music here: send it to the phone; music on the phone (or none anywhere): ask the phone for it
      const pull = !playingHere && (this.plays(p) || !store.snapshot?.track);
      await api.devicesTransfer(p.dev, pull);
      this.setOpen(false);
    } catch (e) {
      toast(errorMessage(e), 'error');
    }
  }

  private row(o: { name: string; sub: string; kind: string; active: boolean; enabled?: boolean; onClick: (() => void) | null; volume?: DeviceInfo }): HTMLElement {
    const on = o.enabled !== false;
    const r = h(
      'button',
      { type: 'button', class: `dv-row${o.active ? ' is-active' : ''}${on ? '' : ' is-off'}`, disabled: !on && !!o.onClick },
      icon(o.kind === 'phone' ? I.phone : I.monitor),
      h('span', { class: 'dv-text' }, h('span', { class: 'strong', text: o.name }), h('span', { class: `small ${o.active ? 'dv-accent' : 'muted'}`, text: o.sub })),
      o.active ? icon(I.check) : h('span'),
    );
    if (o.onClick) r.addEventListener('click', o.onClick);
    else r.classList.add('is-here');
    if (!o.volume) return r;
    // the volume of the phone that plays
    const dev = o.volume;
    const vol = h('input', { type: 'range', min: '0', max: '100', class: 'dv-vol', 'aria-label': T('Громкость') }) as HTMLInputElement;
    vol.value = String(Math.round((dev.remote?.vol ?? 1) * 100));
    vol.addEventListener('change', () => void api.devicesCommand(dev.dev, 'vol', Number(vol.value) / 100).catch((e) => toast(errorMessage(e), 'error')));
    return h('div', { class: 'dv-wrap' }, r, vol);
  }
}
