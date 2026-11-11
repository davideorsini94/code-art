<script lang="ts"></script>

<header class="bar" data-tauri-drag-region>
  <div class="traffic-space" data-tauri-drag-region></div>
  <div class="proj" data-tauri-drag-region>Code Art</div>
  <div class="pal" data-tauri-drag-region>⌘K &nbsp; Cerca file, conversazioni, comandi…</div>
</header>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 14px;
    height: 44px;
    padding: 0 14px;
    border-bottom: 1px solid var(--edge-soft);
  }
  .traffic-space { width: 70px; flex: none; height: 100%; }
  .proj { font-size: 15px; font-weight: 650; letter-spacing: -0.01em; }
  .pal {
    margin: 0 auto;
    width: 340px;
    padding: 6px 14px;
    border-radius: 999px;
    color: var(--faint);
    background: var(--glass);
    border: 1px solid var(--edge-soft);
    text-align: center;
    font-size: 12px;
  }
</style>
