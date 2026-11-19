"""Pinned public qualification tools; Linux amd64 only. No credentials."""
import hashlib,io,pathlib,sys,tarfile,urllib.request
pins={
 'gitleaks':('gitleaks/gitleaks','v8.30.1','gitleaks_8.30.1_linux_x64.tar.gz','551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb'),
 'syft':('anchore/syft','v1.57.2','syft_1.57.2_linux_amd64.tar.gz','ad92bd3ec1b94a3df01d4d0aec199afa9c42b72e86da436ff5f837914d32706b'),
 'grype':('anchore/grype','v0.122.1','grype_0.122.1_linux_amd64.tar.gz','f1ba7e2917ee1395618de20a98da0aa7c0fb1788927797651047c50502a37538')}
repo,version,name,digest=pins[sys.argv[1]];out=pathlib.Path(sys.argv[2]);out.mkdir(parents=True,exist_ok=True)
data=urllib.request.urlopen(f'https://github.com/{repo}/releases/download/{version}/{name}',timeout=120).read();assert hashlib.sha256(data).hexdigest()==digest
with tarfile.open(fileobj=io.BytesIO(data),mode='r:gz') as archive:archive.extractall(out,filter='data')
