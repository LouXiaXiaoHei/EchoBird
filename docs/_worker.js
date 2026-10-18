const INSTALLERS = {
  windows: 'Windows_x64-setup.exe',
  'macos-arm': 'macOS_arm64.dmg',
  'linux-deb': 'Linux_x64.deb',
};

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (!url.pathname.startsWith('/download/')) return env.ASSETS.fetch(request);

    const platform = url.pathname.slice('/download/'.length);
    const installer = Object.hasOwn(INSTALLERS, platform) ? INSTALLERS[platform] : null;
    if (!installer) return new Response('Unknown download platform.', { status: 404 });
    if (request.method !== 'GET' && request.method !== 'HEAD') {
      return new Response('Method not allowed.', { status: 405, headers: { Allow: 'GET, HEAD' } });
    }

    try {
      const manifest = await env.ASSETS.fetch(new Request(new URL('/api/version/index.json', url)));
      if (!manifest.ok) throw new Error('Version manifest unavailable');
      const { version } = await manifest.json();
      if (!version) throw new Error('Version missing');

      const filename = `EchoBird_${version}_${installer}`;
      const response = await fetch(
        `https://github.com/edison7009/EchoBird/releases/download/v${version}/${filename}`,
        { method: request.method, headers: { 'User-Agent': 'EchoBird-Download' } }
      );
      if (!response.ok) {
        return new Response('Installer unavailable. Please try again later.', {
          status: response.status,
        });
      }

      const headers = new Headers({
        'Content-Type': 'application/octet-stream',
        'Content-Disposition': `attachment; filename="${filename}"`,
        'Cache-Control': 'no-store',
      });
      const length = response.headers.get('Content-Length');
      if (length) headers.set('Content-Length', length);
      return new Response(request.method === 'HEAD' ? null : response.body, { headers });
    } catch {
      return new Response('Download temporarily unavailable. Please try again later.', {
        status: 503,
      });
    }
  },
};
