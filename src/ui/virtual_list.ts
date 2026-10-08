// Fixed-height virtualized list: only visible rows (+overscan) exist in the DOM,
// rows are recycled on scroll. Handles tens of thousands of likes.

export interface RowSource<T> {
  count(): number;
  get(index: number): T | undefined;
  /** Load whatever is missing in [from, to). Resolves when data is available. */
  ensure(from: number, to: number): Promise<void>;
}

export interface RowRenderer<T> {
  create(): HTMLElement;
  update(row: HTMLElement, item: T | undefined, index: number): void;
}

const OVERSCAN = 6;

export class VirtualList<T> {
  readonly el: HTMLDivElement;
  private spacer: HTMLDivElement;
  private pool: HTMLElement[] = [];
  private frame = 0;
  private loading = false;
  private onActivate?: (index: number) => void;

  constructor(
    private source: RowSource<T>,
    private renderer: RowRenderer<T>,
    private rowHeight: number,
  ) {
    this.el = document.createElement('div');
    this.el.className = 'vlist';
    this.el.tabIndex = 0;
    this.el.setAttribute('role', 'list');
    this.spacer = document.createElement('div');
    this.spacer.className = 'vlist-spacer';
    this.el.append(this.spacer);

    this.el.addEventListener('scroll', () => this.schedule(), { passive: true });
    new ResizeObserver(() => this.schedule()).observe(this.el);
    this.el.addEventListener('dblclick', (e) => {
      const i = this.indexOf(e.target);
      if (i !== null) this.onActivate?.(i);
    });
    this.el.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        const i = this.indexOf(document.activeElement);
        if (i !== null) this.onActivate?.(i);
      }
    });
  }

  onRowActivate(cb: (index: number) => void): void {
    this.onActivate = cb;
  }

  setSource(source: RowSource<T>): void {
    this.source = source;
    this.el.scrollTop = 0;
    this.refresh();
  }

  /** Re-render visible rows (after data or "current track" changes). */
  refresh(): void {
    this.schedule();
  }

  private indexOf(target: EventTarget | null): number | null {
    const row = (target as HTMLElement | null)?.closest?.('[data-index]') as HTMLElement | null;
    if (!row || !this.el.contains(row)) return null;
    const i = Number(row.dataset.index);
    return Number.isFinite(i) ? i : null;
  }

  private schedule(): void {
    if (this.frame) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      this.render();
    });
  }

  private render(): void {
    const count = this.source.count();
    const rh = this.rowHeight;
    this.spacer.style.height = `${count * rh}px`;

    const viewport = this.el.clientHeight || 600;
    const first = Math.max(0, Math.floor(this.el.scrollTop / rh) - OVERSCAN);
    const last = Math.min(count, Math.ceil((this.el.scrollTop + viewport) / rh) + OVERSCAN);
    const needed = Math.max(0, last - first);

    while (this.pool.length < needed) {
      const row = this.renderer.create();
      row.style.position = 'absolute';
      row.style.left = '0';
      row.style.right = '0';
      row.style.height = `${rh}px`;
      row.setAttribute('role', 'listitem');
      row.tabIndex = -1;
      this.pool.push(row);
      this.el.append(row);
    }

    let missing = false;
    this.pool.forEach((row, k) => {
      const i = first + k;
      if (k >= needed) {
        row.hidden = true;
        return;
      }
      row.hidden = false;
      row.dataset.index = String(i);
      row.style.transform = `translateY(${i * rh}px)`;
      const item = this.source.get(i);
      if (item === undefined) missing = true;
      this.renderer.update(row, item, i);
    });

    if ((missing || last >= count - OVERSCAN) && !this.loading) {
      this.loading = true;
      this.source
        .ensure(first, last)
        .catch(() => undefined)
        .finally(() => {
          this.loading = false;
          if (missing) this.schedule();
        });
    }
  }
}

/** Array-backed source with optional "load more" at the end (search results). */
export class ArraySource<T> implements RowSource<T> {
  constructor(
    public items: T[] = [],
    private loadMore?: () => Promise<boolean>,
  ) {}
  count(): number {
    return this.items.length;
  }
  get(i: number): T | undefined {
    return this.items[i];
  }
  async ensure(_from: number, to: number): Promise<void> {
    if (this.loadMore && to >= this.items.length - 10) await this.loadMore();
  }
}

/** Page cache over an offset/limit backend (likes from SQLite). */
export class PagedSource<T> implements RowSource<T> {
  private pages = new Map<number, T[]>();
  private pending = new Map<number, Promise<void>>();

  constructor(
    private total: number,
    private fetchPage: (offset: number, limit: number) => Promise<T[]>,
    private pageSize = 200,
  ) {}

  count(): number {
    return this.total;
  }

  get(i: number): T | undefined {
    return this.pages.get(Math.floor(i / this.pageSize))?.[i % this.pageSize];
  }

  async ensure(from: number, to: number): Promise<void> {
    const firstPage = Math.floor(from / this.pageSize);
    const lastPage = Math.floor(Math.max(from, to - 1) / this.pageSize);
    const loads: Promise<void>[] = [];
    for (let p = firstPage; p <= lastPage; p++) {
      if (this.pages.has(p) || p * this.pageSize >= this.total) continue;
      let job = this.pending.get(p);
      if (!job) {
        job = this.fetchPage(p * this.pageSize, this.pageSize)
          .then((rows) => {
            this.pages.set(p, rows);
          })
          .finally(() => this.pending.delete(p));
        this.pending.set(p, job);
      }
      loads.push(job);
    }
    await Promise.all(loads);
  }
}
