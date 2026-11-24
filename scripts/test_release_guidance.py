import unittest

from check_release_guidance import observe, classify


def fixture(version='1.1.6', pin=None):
    names = [
        f'SHA256SUMS-{version}{s}.txt'
        for s in ('', '-windows', '-linux')
    ]

    release = dict(
        tag_name='v' + version,
        name='NIGHTFALLCOIN Core ' + version,
        draft=False,
        prerelease=False,
        assets=[dict(name=n) for n in names]
    )

    home = (
        '<h2>Core ' + version + '</h2>' +
        ' '.join(names)
    )

    build = (
        '<pre>git checkout v' +
        (pin or version) +
        '</pre>'
    )

    return release, home, build


class GuidanceTests(unittest.TestCase):

    def test_consistent(self):
        result = observe(*fixture())
        self.assertEqual(classify(result, result), 0)

    def test_next_release(self):
        result = observe(*fixture('1.1.10'))
        self.assertEqual(result['findings'], [])

    def test_old_checkout(self):
        result = observe(*fixture(pin='1.0.12'))
        self.assertEqual(classify(result, result), 1)
        self.assertTrue(any(
            'activation floor' in x
            for x in result['findings']
        ))

    def test_old_but_compatible(self):
        result = observe(*fixture(pin='1.1.5'))
        self.assertEqual(len(result['findings']), 1)

    def test_changed_observation(self):
        self.assertEqual(
            classify(
                observe(*fixture()),
                observe(*fixture(pin='1.0.12'))
            ),
            2
        )

    def test_missing_manifest(self):
        release, home, build = fixture()
        release['assets'].pop()
        self.assertTrue(
            observe(release, home, build)['findings']
        )

    def test_missing_home_manifest(self):
        release, home, build = fixture()
        home = home.replace(
            'SHA256SUMS-1.1.6-linux.txt',
            ''
        )
        self.assertTrue(
            observe(release, home, build)['findings']
        )

    def test_missing_checkout(self):
        release, home, _ = fixture()
        with self.assertRaises(ValueError):
            observe(
                release,
                home,
                '<p>maintenance</p>'
            )

    def test_ambiguous_checkout(self):
        release, home, build = fixture()
        with self.assertRaises(ValueError):
            observe(
                release,
                home,
                build + '<pre>git checkout v1.1.3</pre>'
            )

    def test_ignore_script(self):
        release, home, build = fixture()
        home += '<script>Core 0.1.2</script>'
        self.assertEqual(
            observe(release, home, build)['findings'],
            []
        )

    def test_web_release(self):
        release, home, build = fixture()
        release['tag_name'] = 'web-v1.1.6'
        with self.assertRaises(ValueError):
            observe(release, home, build)

    def test_prerelease(self):
        release, home, build = fixture()
        release['prerelease'] = True
        with self.assertRaises(ValueError):
            observe(release, home, build)


if __name__ == '__main__':
    unittest.main()
