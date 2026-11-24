#!/usr/bin/env python3
"""Compare published release guidance, not binary integrity or chain state."""

import argparse
import json
import re
import sys
import time
from html.parser import HTMLParser
from urllib.parse import urlsplit
from urllib.request import Request, urlopen

API = 'https://api.github.com/repos/Instinctes/nightfall/releases/latest'
HOME = 'https://nightfallcoin.org/'
BUILD = HOME + 'build/'

VERSION = r'([0-9]+\.[0-9]+\.[0-9]+)'
FLOOR = (1, 1, 1)
LIMIT = 4 * 1024 * 1024


class Text(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.parts = []
        self.hidden = 0

    def handle_starttag(self, tag, attrs):
        if tag in ('script', 'style', 'template'):
            self.hidden += 1

    def handle_endtag(self, tag):
        if tag in ('script', 'style', 'template') and self.hidden:
            self.hidden -= 1

    def handle_data(self, data):
        if not self.hidden:
            self.parts.append(data)


def text(html):
    parser = Text()
    parser.feed(html)
    parser.close()
    return ' '.join(' '.join(parser.parts).split())


def get(url):
    req = Request(
        url,
        headers={
            'User-Agent': 'Nightfall-release-guidance/1',
            'Cache-Control': 'no-cache'
        }
    )

    with urlopen(req, timeout=20) as res:
        if urlsplit(res.geturl()).netloc != urlsplit(url).netloc:
            raise ValueError('unexpected redirect host')

        if urlsplit(res.geturl()).scheme != 'https':
            raise ValueError('unexpected redirect scheme')

        raw = res.read(LIMIT + 1)

    if len(raw) > LIMIT:
        raise ValueError('response too large')

    return raw.decode('utf-8')


def version_tuple(s):
    return tuple(map(int, s.split('.')))


def observe(release, homepage, buildpage):
    if (
        not isinstance(release, dict)
        or release.get('draft') is not False
        or release.get('prerelease') is not False
    ):
        raise ValueError('not a stable public release')

    tag = re.fullmatch(
        'v' + VERSION,
        release.get('tag_name', '')
    )

    if not tag or 'Core' not in release.get('name', ''):
        raise ValueError(
            'latest release is not a recognized Core release'
        )

    latest = tag.group(1)
    home = text(homepage)
    build = text(buildpage)

    current = sorted(set(re.findall(
        r'\bCore(?: Wallet)?\s+' + VERSION + r'\b',
        home
    )))

    pins = sorted(set(re.findall(
        r'\bgit checkout\s+(?:--detach\s+)?v' + VERSION + r'\b',
        build
    )))

    if len(current) != 1 or len(pins) != 1:
        raise ValueError('missing or ambiguous release guidance')

    manifests = [
        f'SHA256SUMS-{latest}{suffix}.txt'
        for suffix in ('', '-windows', '-linux')
    ]

    assets = release.get('assets')

    if not isinstance(assets, list) or not assets:
        raise ValueError('release assets unavailable')

    names = {asset['name'] for asset in assets}
    findings = []

    for location, value in [
        ('home Core', current[0]),
        ('build checkout', pins[0])
    ]:
        if value != latest:
            findings.append(
                f'{location}: {value} != published Core {latest}'
            )

        if version_tuple(value) < FLOOR:
            findings.append(
                f'{location}: below documented activation floor 1.1.1'
            )

    for name in manifests:
        if name not in names:
            findings.append(
                f'latest release asset absent: {name}'
            )

        if name not in home:
            findings.append(
                f'homepage manifest reference absent: {name}'
            )

    return {
        'latest': latest,
        'home': current[0],
        'checkout': pins[0],
        'findings': findings
    }


def snapshot():
    return observe(
        json.loads(get(API)),
        get(HOME),
        get(BUILD)
    )


def classify(first, second):
    if first != second:
        return 2
    return 1 if first['findings'] else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        '--report',
        help='write JSON to this file'
    )
    args = parser.parse_args()

    report = {
        'scope': 'public release guidance only'
    }

    try:
        first = snapshot()
        time.sleep(2)
        second = snapshot()
        status = classify(first, second)
        report.update(first=first, second=second)

    except (
        OSError,
        ValueError,
        TypeError,
        KeyError,
        AttributeError
    ) as exc:
        status = 2
        report['error'] = type(exc).__name__

    report['status'] = (
        'CONSISTENT',
        'DRIFT',
        'INCONCLUSIVE'
    )[status]

    output = json.dumps(
        report,
        indent=2,
        sort_keys=True
    ) + '\n'

    print(output, end='')

    if args.report:
        with open(
            args.report,
            'w',
            encoding='utf-8'
        ) as f:
            f.write(output)

    return status


if __name__ == '__main__':
    sys.exit(main())
