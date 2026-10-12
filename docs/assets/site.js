// The story (the screenshot and the text change while the page is scrolled) and the download button
// (the latest version and the installer's address come from GitHub).
(function () {
  'use strict';

  // ---------------------------------------------------------------- the story
  var story = document.getElementById('story');
  if (story) {
    var caps = story.querySelectorAll('.cap');
    var shots = story.querySelectorAll('.shot');
    var ticks = story.querySelectorAll('.tick');
    var cursor = story.querySelector('.cursor');
    var current = -1;
    var clickTimer = 0;

    var show = function (step) {
      if (step === current) return;
      current = step;
      for (var i = 0; i < caps.length; i++) {
        caps[i].classList.toggle('is-on', i === step);
        shots[i].classList.toggle('is-on', i === step);
        ticks[i].classList.toggle('is-on', i === step);
      }
      // the cursor goes to the button that matters on this screen, then clicks
      var s = shots[step];
      cursor.classList.remove('click');
      window.clearTimeout(clickTimer);
      cursor.style.left = s.dataset.x + '%';
      cursor.style.top = s.dataset.y + '%';
      clickTimer = window.setTimeout(function () {
        cursor.classList.remove('click');
        void cursor.offsetWidth;
        cursor.classList.add('click');
      }, 1050);
    };

    var ticking = false;
    var update = function () {
      ticking = false;
      var r = story.getBoundingClientRect();
      var span = story.offsetHeight - window.innerHeight;
      var p = span > 0 ? Math.min(1, Math.max(0, -r.top / span)) : 0;
      show(Math.min(caps.length - 1, Math.floor(p * caps.length)));
    };
    var onScroll = function () {
      if (!ticking) {
        ticking = true;
        window.requestAnimationFrame(update);
      }
    };
    window.addEventListener('scroll', onScroll, { passive: true });
    window.addEventListener('resize', onScroll);
    update();
  }

  // ---------------------------------------------------------------- download
  var REPO = 'b1rkarokK/SC-Desktop';
  var links = document.querySelectorAll('[data-download]');

  var apply = function (rel) {
    var asset = (rel.assets || []).filter(function (a) {
      return /x64-setup\.exe$/i.test(a.name);
    })[0];
    if (!asset) return;
    var version = String(rel.tag_name || '').replace(/^v/, '');
    links.forEach(function (a) {
      a.href = asset.browser_download_url;
      a.setAttribute('download', asset.name);
      var v = a.querySelector('.ver');
      if (v && version) v.textContent = 'v' + version;
    });
    var ru = document.documentElement.lang === 'ru';
    var mb = (asset.size / 1048576).toFixed(1);
    if (ru) mb = mb.replace('.', ',');
    var sz = document.querySelector('.meta-size');
    if (sz) sz.textContent = version ? 'v' + version + ' · ' + mb + ' ' + (ru ? 'МБ' : 'MB') : '';
    var facts = document.querySelector('.facts');
    if (facts) {
      facts.closest('details').hidden = false;
      facts.querySelector('.f-ver').textContent = version;
      facts.querySelector('.f-size').textContent = mb;
      var hash = String(asset.digest || '').replace(/^sha256:/, '');
      var h = facts.querySelector('.f-hash');
      if (hash) h.textContent = hash;
      else h.parentNode.hidden = true;
    }
  };

  var cached = null;
  try {
    cached = JSON.parse(sessionStorage.getItem('scdesk-release') || 'null');
  } catch (e) {
    /* storage may be blocked */
  }
  if (cached) apply(cached);
  else {
    fetch('https://api.github.com/repos/' + REPO + '/releases/latest', { headers: { Accept: 'application/vnd.github+json' } })
      .then(function (r) {
        return r.ok ? r.json() : Promise.reject(new Error(String(r.status)));
      })
      .then(function (rel) {
        try {
          sessionStorage.setItem('scdesk-release', JSON.stringify(rel));
        } catch (e) {
          /* ignore */
        }
        apply(rel);
      })
      .catch(function () {
        /* the links keep pointing at the releases page */
      });
  }
})();
