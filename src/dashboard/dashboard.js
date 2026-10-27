(function () {
  'use strict';

  var PAGE_SIZE = 10;
  var STORAGE_THEME = 'gyroxide.theme';
  var STORAGE_LANG = 'gyroxide.lang';

  var MESSAGES = {
    id: {
      'brand.subtitle': 'Proxy console',
      'nav.monitoring': 'Monitoring',
      'lang.label': 'Bahasa',
      'theme.toggle': 'Ganti tema',

      'stats.total': 'Total',
      'stats.totalNote': 'proxy terdaftar',
      'stats.alive': 'Alive',
      'stats.aliveNote': 'siap dipakai',
      'stats.dead': 'Dead',
      'stats.deadNote': 'perlu pemeriksaan',
      'stats.availability': 'Ketersediaan',
      'stats.availabilityNote': 'dari total proxy',

      'panel.title': 'Daftar proxy',
      'panel.desc': 'Endpoint upstream yang dipakai untuk rotasi permintaan.',
      'bulk.open': 'Unggah massal',
      'bulk.close': 'Tutup',

      'search.placeholder': 'Cari alamat, port, atau status…',
      'search.label': 'Cari proxy',
      'filter.protocol': 'Filter protokol',
      'filter.protocolAll': 'Semua protokol',
      'filter.status': 'Filter status',
      'filter.statusAll': 'Semua status',
      'filter.alive': 'Hidup',
      'filter.dead': 'Mati',
      'quick.label': 'Tambah proxy cepat',
      'quick.button': 'Tambah',

      'selection.countLabel': 'proxy dipilih',
      'selection.selectAll': 'Pilih semua hasil',
      'selection.copy': 'Salin',
      'selection.export': 'Ekspor .txt',
      'selection.delete': 'Hapus',
      'selection.clear': 'Batal',
      'selectAll.label': 'Pilih semua di halaman ini',

      'th.no': 'No.',
      'th.address': 'Alamat',
      'th.protocol': 'Protokol',
      'th.auth': 'Autentikasi',
      'th.status': 'Status',
      'th.fails': 'Gagal',
      'th.actions': 'Aksi',

      'status.alive': 'Hidup',
      'status.dead': 'Mati',

      'empty.none.title': 'Belum ada proxy',
      'empty.none.desc': 'Tambahkan proxy lewat kolom di atas atau unggah massal.',
      'empty.search.title': 'Tidak ada hasil',
      'empty.search.desc': 'Sesuaikan kata kunci pencarian atau filter yang aktif.',

      'pagination.nav': 'Navigasi halaman',
      'pagination.prev': 'Halaman sebelumnya',
      'pagination.next': 'Halaman berikutnya',
      'pagination.page': 'Halaman {0}',
      'pagination.info': 'Menampilkan {0}–{1} dari {2} proxy',
      'pagination.infoEmpty': 'Menampilkan 0 dari 0 proxy',

      'tabs.bulk': 'Unggah massal',
      'tabs.raw': 'Editor mentah',
      'bulk.hint': 'Satu proxy per baris. Format yang didukung: <code>ip:port</code>, <code>ip:port:user:pass</code>, <code>http://…</code>, dan <code>socks5://…</code>. Duplikat otomatis dilewati.',
      'bulk.note': 'Proxy baru ditambahkan tanpa menghapus daftar lama.',
      'bulk.submit': 'Tambahkan proxy',
      'raw.callout': 'Menyimpan dari editor ini akan menimpa seluruh isi <code>proxies.txt</code>.',
      'raw.label': 'Isi file proxies.txt',
      'raw.note': 'Gunakan hanya jika Anda perlu mengganti daftar secara keseluruhan.',
      'raw.submit': 'Simpan dan timpa',

      'footer.product': 'Gyroxide · Auto-rotate proxy gateway',
      'footer.metrics': 'Prometheus metrics tersedia di',

      'dialog.title': 'Konfirmasi',
      'dialog.message': 'Lanjutkan tindakan ini?',
      'dialog.cancel': 'Batal',
      'dialog.confirmDefault': 'Lanjutkan',
      'dialog.delete.title': 'Hapus proxy',
      'dialog.delete.one': 'Hapus proxy {0} dari daftar?',
      'dialog.delete.many': 'Hapus {0} proxy dari daftar?',
      'dialog.delete.note': 'Perubahan langsung disimpan ke proxies.txt.',
      'dialog.delete.ok': 'Hapus',
      'dialog.overwrite.title': 'Timpa daftar proxy',
      'dialog.overwrite.message': 'Seluruh isi proxies.txt akan diganti dengan teks pada editor. Tindakan ini tidak bisa dibatalkan.',
      'dialog.overwrite.ok': 'Simpan dan timpa',

      'toast.copied': '{0} proxy disalin',
      'toast.copyFailed': 'Gagal menyalin ke clipboard',
      'toast.proxyCopied': 'Proxy disalin',
      'toast.deleteNotFound': 'Proxy tidak ditemukan pada daftar, muat ulang halaman',
      'toast.noneSelected': 'Belum ada proxy yang dipilih',
      'toast.fillProxy': 'Isi proxy terlebih dahulu',
      'toast.nothingToAdd': 'Belum ada proxy untuk ditambahkan',

      'alert.replaceOk': 'Berhasil! {0} proxy disimpan dan di-reload.',
      'alert.replaceErr': 'Gagal memperbarui: {0}',
      'alert.appendOk': 'Berhasil menambahkan {0} proxy baru! (Total sekarang: {1} proxy)',
      'alert.appendNone': 'Tidak ada proxy baru yang ditambahkan (semua proxy sudah ada dalam daftar atau format tidak valid). Total: {0} proxy',
      'alert.appendErr': 'Gagal menambahkan: {0}'
    },

    en: {
      'brand.subtitle': 'Proxy console',
      'nav.monitoring': 'Monitoring',
      'lang.label': 'Language',
      'theme.toggle': 'Toggle theme',

      'stats.total': 'Total',
      'stats.totalNote': 'registered proxies',
      'stats.alive': 'Alive',
      'stats.aliveNote': 'ready to use',
      'stats.dead': 'Dead',
      'stats.deadNote': 'needs attention',
      'stats.availability': 'Availability',
      'stats.availabilityNote': 'of all proxies',

      'panel.title': 'Proxy list',
      'panel.desc': 'Upstream endpoints used for request rotation.',
      'bulk.open': 'Bulk upload',
      'bulk.close': 'Close',

      'search.placeholder': 'Search address, port, or status…',
      'search.label': 'Search proxies',
      'filter.protocol': 'Protocol filter',
      'filter.protocolAll': 'All protocols',
      'filter.status': 'Status filter',
      'filter.statusAll': 'All statuses',
      'filter.alive': 'Alive',
      'filter.dead': 'Dead',
      'quick.label': 'Quick add proxy',
      'quick.button': 'Add',

      'selection.countLabel': 'proxies selected',
      'selection.selectAll': 'Select all results',
      'selection.copy': 'Copy',
      'selection.export': 'Export .txt',
      'selection.delete': 'Delete',
      'selection.clear': 'Clear',
      'selectAll.label': 'Select all on this page',

      'th.no': 'No.',
      'th.address': 'Address',
      'th.protocol': 'Protocol',
      'th.auth': 'Auth',
      'th.status': 'Status',
      'th.fails': 'Fails',
      'th.actions': 'Actions',

      'status.alive': 'Alive',
      'status.dead': 'Dead',

      'empty.none.title': 'No proxies yet',
      'empty.none.desc': 'Add proxies with the field above or use bulk upload.',
      'empty.search.title': 'No results',
      'empty.search.desc': 'Adjust the search term or the active filters.',

      'pagination.nav': 'Pagination',
      'pagination.prev': 'Previous page',
      'pagination.next': 'Next page',
      'pagination.page': 'Page {0}',
      'pagination.info': 'Showing {0}–{1} of {2} proxies',
      'pagination.infoEmpty': 'Showing 0 of 0 proxies',

      'tabs.bulk': 'Bulk upload',
      'tabs.raw': 'Raw editor',
      'bulk.hint': 'One proxy per line. Supported formats: <code>ip:port</code>, <code>ip:port:user:pass</code>, <code>http://…</code>, and <code>socks5://…</code>. Duplicates are skipped automatically.',
      'bulk.note': 'New proxies are added without removing the existing list.',
      'bulk.submit': 'Add proxies',
      'raw.callout': 'Saving from this editor overwrites the whole <code>proxies.txt</code>.',
      'raw.label': 'proxies.txt contents',
      'raw.note': 'Use only when you need to replace the entire list.',
      'raw.submit': 'Save and overwrite',

      'footer.product': 'Gyroxide · Auto-rotate proxy gateway',
      'footer.metrics': 'Prometheus metrics available at',

      'dialog.title': 'Confirm',
      'dialog.message': 'Continue with this action?',
      'dialog.cancel': 'Cancel',
      'dialog.confirmDefault': 'Continue',
      'dialog.delete.title': 'Delete proxy',
      'dialog.delete.one': 'Remove proxy {0} from the list?',
      'dialog.delete.many': 'Remove {0} proxies from the list?',
      'dialog.delete.note': 'Changes are saved to proxies.txt immediately.',
      'dialog.delete.ok': 'Delete',
      'dialog.overwrite.title': 'Overwrite proxy list',
      'dialog.overwrite.message': 'The entire proxies.txt content will be replaced with the text in the editor. This cannot be undone.',
      'dialog.overwrite.ok': 'Save and overwrite',

      'toast.copied': '{0} proxies copied',
      'toast.copyFailed': 'Failed to copy to clipboard',
      'toast.proxyCopied': 'Proxy copied',
      'toast.deleteNotFound': 'Proxy not found in the list, reload the page',
      'toast.noneSelected': 'No proxies selected',
      'toast.fillProxy': 'Enter a proxy first',
      'toast.nothingToAdd': 'No proxies to add',

      'alert.replaceOk': 'Done! {0} proxies saved and reloaded.',
      'alert.replaceErr': 'Update failed: {0}',
      'alert.appendOk': 'Added {0} new proxies! (Total: {1} proxies)',
      'alert.appendNone': 'No new proxies added (all already exist or the format is invalid). Total: {0} proxies',
      'alert.appendErr': 'Add failed: {0}'
    }
  };

  var currentPage = 1;
  var allRows = [];
  var selected = new Map();
  var toastTimer = null;
  var lang = document.documentElement.lang === 'en' ? 'en' : 'id';

  function $(id) { return document.getElementById(id); }

  function t(key, args) {
    var dict = MESSAGES[lang] || MESSAGES.id;
    var str = dict[key];
    if (str === undefined) str = MESSAGES.id[key];
    if (str === undefined) return key;
    if (!args) return str;
    return str.replace(/\{(\d+)\}/g, function (match, index) {
      var value = args[Number(index)];
      return value === undefined ? match : String(value);
    });
  }

  function rowsFromDom() {
    return Array.prototype.slice.call(
      document.querySelectorAll('#proxyTableBody tr.proxy-row')
    );
  }

  function showToast(message, isError) {
    var el = $('toastNotification');
    if (!el) return;
    el.textContent = message;
    el.className = 'toast show' + (isError ? ' error' : '');
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(function () {
      el.className = 'toast' + (isError ? ' error' : '');
    }, 2600);
  }

  /* ---------------------------------------------------------------- theme */

  function currentTheme() {
    return document.documentElement.getAttribute('data-theme') === 'dark' ? 'dark' : 'light';
  }

  function applyThemeState() {
    var btn = $('themeToggle');
    if (!btn) return;
    var dark = currentTheme() === 'dark';
    btn.setAttribute('aria-pressed', dark ? 'true' : 'false');
    btn.setAttribute('aria-label', t('theme.toggle'));
    btn.setAttribute('title', t('theme.toggle'));
  }

  function toggleTheme() {
    var next = currentTheme() === 'dark' ? 'light' : 'dark';
    document.documentElement.setAttribute('data-theme', next);
    try { localStorage.setItem(STORAGE_THEME, next); } catch (e) {}
    applyThemeState();
  }

  /* ------------------------------------------------------------- language */

  function updateLangButtons() {
    var idBtn = $('langIdBtn');
    var enBtn = $('langEnBtn');
    if (idBtn) idBtn.setAttribute('aria-pressed', lang === 'id' ? 'true' : 'false');
    if (enBtn) enBtn.setAttribute('aria-pressed', lang === 'en' ? 'true' : 'false');
  }

  function updateRowStatusLabels() {
    for (var i = 0; i < allRows.length; i++) {
      var row = allRows[i];
      var text = row.querySelector('.status-text');
      if (!text) continue;
      text.textContent = row.dataset.status === 'alive' ? t('status.alive') : t('status.dead');
    }
  }

  function updateBulkLabel() {
    var section = $('bulkSection');
    var label = $('toggleBulkLabel');
    if (!section || !label) return;
    label.textContent = section.hidden ? t('bulk.open') : t('bulk.close');
  }

  function translateAlert() {
    var el = document.querySelector('[data-alert-key]');
    if (!el) return;
    var span = el.querySelector('span');
    if (!span) return;
    var args = [];
    for (var i = 0; i < 4; i++) {
      var value = el.getAttribute('data-alert-arg' + i);
      if (value === null) break;
      args.push(value);
    }
    span.textContent = t(el.getAttribute('data-alert-key'), args);
  }

  function applyTranslations() {
    document.documentElement.lang = lang;

    var nodes = document.querySelectorAll('[data-i18n]');
    for (var i = 0; i < nodes.length; i++) {
      nodes[i].textContent = t(nodes[i].getAttribute('data-i18n'));
    }

    nodes = document.querySelectorAll('[data-i18n-html]');
    for (var j = 0; j < nodes.length; j++) {
      nodes[j].innerHTML = t(nodes[j].getAttribute('data-i18n-html'));
    }

    nodes = document.querySelectorAll('[data-i18n-placeholder]');
    for (var k = 0; k < nodes.length; k++) {
      nodes[k].placeholder = t(nodes[k].getAttribute('data-i18n-placeholder'));
    }

    nodes = document.querySelectorAll('[data-i18n-aria-label]');
    for (var l = 0; l < nodes.length; l++) {
      nodes[l].setAttribute('aria-label', t(nodes[l].getAttribute('data-i18n-aria-label')));
    }

    nodes = document.querySelectorAll('[data-i18n-title]');
    for (var m = 0; m < nodes.length; m++) {
      nodes[m].setAttribute('title', t(nodes[m].getAttribute('data-i18n-title')));
    }

    updateRowStatusLabels();
    updateBulkLabel();
    updateLangButtons();
    translateAlert();
  }

  function setLanguage(next) {
    if (next !== 'id' && next !== 'en') return;
    lang = next;
    try { localStorage.setItem(STORAGE_LANG, next); } catch (e) {}
    applyTranslations();
    applyThemeState();
    render(currentPage);
  }

  /* ---------------------------------------------------------------- table */

  function getFilteredRows() {
    var q = ($('searchInput').value || '').trim().toLowerCase();
    var proto = $('protoFilter').value;
    var status = $('statusFilter').value;

    return allRows.filter(function (row) {
      if (proto !== 'all' && row.dataset.proto !== proto) return false;
      if (status !== 'all' && row.dataset.status !== status) return false;
      if (!q) return true;

      var statusLabel = row.dataset.status === 'alive' ? t('status.alive') : t('status.dead');
      var haystack = [
        row.dataset.addr,
        row.dataset.proto,
        row.dataset.status,
        row.dataset.user,
        statusLabel
      ].join(' ').toLowerCase();

      return haystack.indexOf(q) !== -1;
    });
  }

  function buildPageList(page, pages) {
    var out = [];
    if (pages <= 7) {
      for (var i = 1; i <= pages; i++) out.push(i);
      return out;
    }
    out.push(1);
    if (page > 4) out.push('…');
    var from = Math.max(2, page - 1);
    var to = Math.min(pages - 1, page + 1);
    for (var p = from; p <= to; p++) out.push(p);
    if (page < pages - 3) out.push('…');
    out.push(pages);
    return out;
  }

  function renderPagination(page, pages) {
    var list = buildPageList(page, pages);
    var html = '';
    for (var i = 0; i < list.length; i++) {
      var item = list[i];
      if (item === '…') {
        html += '<span class="page-ellipsis">…</span>';
      } else {
        html += '<button type="button" class="page-btn" data-page="' + item + '"' +
          (item === page ? ' aria-current="page"' : '') +
          ' aria-label="' + t('pagination.page', [item]) + '">' + item + '</button>';
      }
    }
    $('paginationPages').innerHTML = html;
    $('prevPageBtn').disabled = page <= 1;
    $('nextPageBtn').disabled = page >= pages;
  }

  function render(page) {
    var filtered = getFilteredRows();
    var total = filtered.length;
    var pages = Math.max(1, Math.ceil(total / PAGE_SIZE));
    var target = page || 1;
    if (target > pages) target = pages;
    if (target < 1) target = 1;
    currentPage = target;

    var start = (currentPage - 1) * PAGE_SIZE;
    var end = Math.min(start + PAGE_SIZE, total);

    for (var i = 0; i < allRows.length; i++) {
      allRows[i].hidden = true;
    }

    for (var j = start; j < end; j++) {
      var row = filtered[j];
      row.hidden = false;
      row.classList.toggle('selected', selected.has(row.dataset.addr));
      var numCell = row.querySelector('.col-num');
      if (numCell) numCell.textContent = String(j + 1);
    }

    var emptyRow = $('emptyRow');
    if (emptyRow) {
      if (total === 0) {
        emptyRow.hidden = false;
        var hasAny = allRows.length > 0;
        $('emptyTitle').textContent = hasAny ? t('empty.search.title') : t('empty.none.title');
        $('emptyDesc').textContent = hasAny ? t('empty.search.desc') : t('empty.none.desc');
      } else {
        emptyRow.hidden = true;
      }
    }

    $('paginationInfo').textContent = total === 0
      ? t('pagination.infoEmpty')
      : t('pagination.info', [start + 1, end, total]);

    renderPagination(currentPage, pages);
    updateSelectAllCheckboxState();
  }

  function updateSelectAllCheckboxState() {
    var box = $('selectAllCheckbox');
    if (!box) return;
    var visible = allRows.filter(function (row) { return !row.hidden; });
    var checked = visible.filter(function (row) {
      var cb = row.querySelector('.row-checkbox');
      return cb && cb.checked;
    });
    box.checked = visible.length > 0 && checked.length === visible.length;
    box.indeterminate = checked.length > 0 && checked.length < visible.length;
  }

  function updateSelectionUI() {
    var count = selected.size;
    $('selectedCountText').textContent = String(count);
    $('selectionBar').hidden = count === 0;
    for (var i = 0; i < allRows.length; i++) {
      var row = allRows[i];
      var cb = row.querySelector('.row-checkbox');
      var isSelected = selected.has(row.dataset.addr);
      if (cb) cb.checked = isSelected;
      row.classList.toggle('selected', isSelected && !row.hidden);
    }
    updateSelectAllCheckboxState();
  }

  function handleRowCheckboxChange(cb) {
    var row = cb.closest('tr');
    if (!row) return;
    if (cb.checked) {
      selected.set(row.dataset.addr, row.dataset.raw);
    } else {
      selected.delete(row.dataset.addr);
    }
    updateSelectionUI();
  }

  function toggleSelectAll(cb) {
    var visible = allRows.filter(function (row) { return !row.hidden; });
    for (var i = 0; i < visible.length; i++) {
      var row = visible[i];
      if (cb.checked) {
        selected.set(row.dataset.addr, row.dataset.raw);
      } else {
        selected.delete(row.dataset.addr);
      }
    }
    updateSelectionUI();
  }

  function selectAllFiltered() {
    var filtered = getFilteredRows();
    for (var i = 0; i < filtered.length; i++) {
      selected.set(filtered[i].dataset.addr, filtered[i].dataset.raw);
    }
    updateSelectionUI();
  }

  function clearAllSelection() {
    selected.clear();
    updateSelectionUI();
  }

  /* ------------------------------------------------------------- actions */

  function copyText(text) {
    if (navigator.clipboard && window.isSecureContext) {
      return navigator.clipboard.writeText(text);
    }
    return new Promise(function (resolve, reject) {
      var ta = document.createElement('textarea');
      ta.value = text;
      ta.setAttribute('readonly', '');
      ta.style.position = 'fixed';
      ta.style.top = '-1000px';
      document.body.appendChild(ta);
      ta.select();
      try {
        if (document.execCommand('copy')) resolve();
        else reject(new Error('copy failed'));
      } catch (e) {
        reject(e);
      } finally {
        document.body.removeChild(ta);
      }
    });
  }

  function copySelectedProxies() {
    if (selected.size === 0) return;
    copyText(Array.from(selected.values()).join('\n'))
      .then(function () { showToast(t('toast.copied', [selected.size])); })
      .catch(function () { showToast(t('toast.copyFailed'), true); });
  }

  function exportSelectedProxies() {
    if (selected.size === 0) return;
    var blob = new Blob([Array.from(selected.values()).join('\n') + '\n'], { type: 'text/plain' });
    var url = URL.createObjectURL(blob);
    var a = document.createElement('a');
    a.href = url;
    a.download = 'proxies-selected.txt';
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    setTimeout(function () { URL.revokeObjectURL(url); }, 1000);
  }

  function copyRowProxy(btn) {
    var row = btn.closest('tr');
    if (!row) return;
    copyText(row.dataset.raw || row.dataset.addr)
      .then(function () { showToast(t('toast.proxyCopied')); })
      .catch(function () { showToast(t('toast.copyFailed'), true); });
  }

  function escapeRegExp(s) {
    return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  }

  function textWithout(raws, addrs) {
    var lines = $('rawEditorTextarea').value.split(/\r?\n/);
    var rawSet = new Set(raws.map(function (r) { return (r || '').trim(); }));
    var patterns = addrs.map(function (a) {
      return new RegExp('(^|[^0-9.])' + escapeRegExp(a) + '(?![0-9])');
    });
    var out = [];
    for (var i = 0; i < lines.length; i++) {
      var line = lines[i];
      var trimmed = line.trim();
      if (!trimmed) { out.push(line); continue; }
      if (rawSet.has(trimmed)) continue;
      var drop = false;
      for (var p = 0; p < patterns.length; p++) {
        if (patterns[p].test(trimmed)) { drop = true; break; }
      }
      if (!drop) out.push(line);
    }
    return { text: out.join('\n'), changed: out.length !== lines.length };
  }

  function submitForm(mode, text) {
    $('formMode').value = mode;
    $('formProxies').value = text;
    $('proxyForm').submit();
  }

  function confirmAction(options) {
    var dlg = $('confirmDialog');
    if (!dlg || typeof dlg.showModal !== 'function') {
      return Promise.resolve(window.confirm(options.message));
    }
    return new Promise(function (resolve) {
      $('confirmTitle').textContent = options.title || t('dialog.title');
      $('confirmMessage').textContent = options.message;
      var okBtn = $('confirmOkBtn');
      okBtn.textContent = options.okLabel || t('dialog.confirmDefault');
      okBtn.className = 'btn ' + (options.danger === false ? 'btn-primary' : 'btn-danger');

      function onClose() {
        dlg.removeEventListener('close', onClose);
        resolve(dlg.returnValue === 'confirm');
      }
      dlg.addEventListener('close', onClose);
      dlg.returnValue = '';
      dlg.showModal();
    });
  }

  function deleteRows(rows) {
    if (!rows.length) return;
    var raws = rows.map(function (r) { return r.dataset.raw; });
    var addrs = rows.map(function (r) { return r.dataset.addr; });
    var result = textWithout(raws, addrs);
    if (!result.changed) {
      showToast(t('toast.deleteNotFound'), true);
      return;
    }
    var label = rows.length === 1
      ? t('dialog.delete.one', [addrs[0]])
      : t('dialog.delete.many', [rows.length]);
    confirmAction({
      title: t('dialog.delete.title'),
      message: label + ' ' + t('dialog.delete.note'),
      okLabel: t('dialog.delete.ok')
    }).then(function (ok) {
      if (ok) submitForm('replace', result.text);
    });
  }

  function deleteRowProxy(btn) {
    var row = btn.closest('tr');
    if (row) deleteRows([row]);
  }

  function deleteSelectedProxies() {
    var rows = allRows.filter(function (row) { return selected.has(row.dataset.addr); });
    if (!rows.length) {
      showToast(t('toast.noneSelected'), true);
      return;
    }
    deleteRows(rows);
  }

  function quickAddProxy() {
    var input = $('quickInput');
    var value = (input.value || '').trim();
    if (!value) {
      input.focus();
      showToast(t('toast.fillProxy'), true);
      return;
    }
    submitForm('append', value);
  }

  function submitBulkUpload() {
    var field = $('bulkUploadInput');
    var value = (field.value || '').trim();
    if (!value) {
      showToast(t('toast.nothingToAdd'), true);
      field.focus();
      return;
    }
    submitForm('append', value);
  }

  function submitRawEditor() {
    var value = $('rawEditorTextarea').value;
    confirmAction({
      title: t('dialog.overwrite.title'),
      message: t('dialog.overwrite.message'),
      okLabel: t('dialog.overwrite.ok')
    }).then(function (ok) {
      if (ok) submitForm('replace', value);
    });
  }

  function toggleBulkEditor() {
    var section = $('bulkSection');
    var open = section.hidden;
    section.hidden = !open;
    $('toggleBulkBtn').setAttribute('aria-expanded', open ? 'true' : 'false');
    updateBulkLabel();
    if (open) $('bulkUploadInput').focus();
  }

  function switchBulkTab(name) {
    var upload = name === 'upload';
    $('tabUploadBtn').classList.toggle('active', upload);
    $('tabRawBtn').classList.toggle('active', !upload);
    $('tabUploadBtn').setAttribute('aria-selected', upload ? 'true' : 'false');
    $('tabRawBtn').setAttribute('aria-selected', upload ? 'false' : 'true');
    $('bulkUploadPane').hidden = !upload;
    $('bulkRawPane').hidden = upload;
  }

  function setupRowClicks() {
    for (var i = 0; i < allRows.length; i++) {
      allRows[i].addEventListener('click', function (e) {
        if (['INPUT', 'BUTTON', 'A', 'CODE', 'TEXTAREA', 'SELECT'].indexOf(e.target.tagName) !== -1) return;
        var cb = this.querySelector('.row-checkbox');
        if (!cb) return;
        cb.checked = !cb.checked;
        handleRowCheckboxChange(cb);
      });
    }
  }

  function init() {
    allRows = rowsFromDom();
    setupRowClicks();
    $('paginationPages').addEventListener('click', function (e) {
      var btn = e.target.closest('button[data-page]');
      if (btn) render(parseInt(btn.dataset.page, 10));
    });
    document.addEventListener('keydown', function (e) {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        $('searchInput').focus();
        $('searchInput').select();
      }
    });
    applyTranslations();
    applyThemeState();
    updateSelectionUI();
    render(1);
  }

  window.render = render;
  window.toggleTheme = toggleTheme;
  window.setLanguage = setLanguage;
  window.handleRowCheckboxChange = handleRowCheckboxChange;
  window.toggleSelectAll = toggleSelectAll;
  window.selectAllFiltered = selectAllFiltered;
  window.clearAllSelection = clearAllSelection;
  window.copySelectedProxies = copySelectedProxies;
  window.exportSelectedProxies = exportSelectedProxies;
  window.deleteSelectedProxies = deleteSelectedProxies;
  window.copyRowProxy = copyRowProxy;
  window.deleteRowProxy = deleteRowProxy;
  window.quickAddProxy = quickAddProxy;
  window.submitBulkUpload = submitBulkUpload;
  window.submitRawEditor = submitRawEditor;
  window.toggleBulkEditor = toggleBulkEditor;
  window.switchBulkTab = switchBulkTab;

  Object.defineProperty(window, 'currentPage', {
    get: function () { return currentPage; },
    configurable: true
  });

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
