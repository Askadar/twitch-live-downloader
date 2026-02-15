# Running

## dependencies
1. reachable `streamlink` executable, refer to https://streamlink.github.io/install.html for installation instructions

## config
when all dependencies are preparred, configure execution params in config.json:
```json
{
	"clientId": "",
	"clientSecret": "",
	"streamlinkToken": "",
	"broadcasters": ["bajiru_en"],
	"root": "/mnt/t/videos/bajiru_en/records"
}
```
details:
 - `clientId` and `clientSecret` are credentials you get from creating an app at https://dev.twitch.tv/console
 - `streamlinkToken` is the auth cookie from twitch site, you need it if you want to skip ads and have an account that either subbed to target channels or have site-wide turbo
 - `broadcasters` is a list of channels you want to watch, currently limited to 5 (will be expanded to 10 and maybe unlimited in the future)
 - `root` is the full path to the folder you want to use to store records; downloader will create sub-folder within of channel login for relevant downloads

 all above options are necessary to fill in at the moment, could change in the future

## auth
after prepping dependencies and config file you will need to auth downloader with twitch, it'll show a prompt `follow the link to auth: https://www.twitch.tv/activate?device-code=******`, open the provided link and auth with twitch site; this will create token.json file with your auth token, there's also going to be occasional validate and refresh token when necessary updaing the token.json file
