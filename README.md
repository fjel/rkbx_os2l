# Rekordbox SoundSwitch (os2l)
Open Sound to Light (os2l) protocol support for Rekordbox.
When run on the same computer as an instance of Rekordbox and SoundSwitch, it will read information from Rekordbox and send to SoundSwitch.

## Features
- Automatically discover SoundSwitch on the local machine.
- Autoloops with beat syncing
- Full autoscript support when using VirtualDJ to prepare the tracks.
- Reads values from rekordbox memory, does not crash or interfere with rekordbox


## Setup
Open SoundSwitch and then go to preferences (cog in upper right corner).  
Under "Input" select "VirtualDJ", and "Auto connect". Restart SoundSwitch.  
I have only tested SoundSwitch and Rekordbox on the same machine, but it might work between computers on the same network.  

## Usage
Start Rekordbox and SoundSwitch, and then start rkbx_os2l.exe with your specific rekordbox version.
`rkbx_os2l.exe [flags]`
where
``` 
 -h  Print help and available versions
 -u  Fetch latest offset list from GitHub and exit
 -v  Rekordbox version to target, eg. 6.8.4

 -p  Change poll value

```
If no arguments are given, it defaults to the latest supported rekordbox version.


### Autoscript
To have autoscript support you need to have VirtualDJ (free version) on your computer to get a proper beatgrid.
Note: The path to the song is what SoundSwitch uses to find the scripted track, so make sure that the VirtualDJ path to the song is the same as the path to the song you use in Rekordbox.
1. Open up VirtualDJ, and find your music in the file browser
2. Right click folder or files and go to "Batch"->"Analyze for BPM etc". This might take a while depending on your computer
3. When VirtualDJ is finished analyzing, go to "Edit" mode in SoundSwitch. Select "Virtual DJ Tracks" in the "Music" menu.
4. Go to a song and select "Automation"->"Autoscript". You can also select multiple tracks, rightclick and "Autoscript selected tracks"
When you go back to performance mode, the lightshow should now be activated when you play the songs from rekordbox (on master).

## Supported versions
I have not started on finding values for version 7 yet.  
Hopefully as stated in https://github.com/grufkork/rkbx_osc i will only need to do this once for version 7.
| Rekordbox Version  |
| ----- |
| 6.8.5 |
| 6.8.4 |

## How it works
By looking at the communication between VirtualDJ and SoundSwitch i was able to find what values were required to have proper autoloop and scripted track support. These values are extracted by reading Rekordbox's memory, and is sent to SoundSwitch using os2l protocol.

## Limitations
- Only supports two decks.
- Only sends data for master deck
- Windows only

# Technical Details

## Offsets file format
The `offsets` file contain the hex memory addresses (without the leading 0x) for the values we need to fetch. The file supports basic comments (# at start of line). Versions are separated by two newlines.

Example entry with explanations:
```
6.8.5                   Rerkordbox Version
052EA410 28 0 48 2468   Deck 1 Bar
052EA410 28 0 48 246C   Deck 1 Beat
052EA410 28 0 50 2468   Deck 2 Bar
052EA410 28 0 50 246C   Deck 2 Beat
0544A460 28 180 0 140   Masterdeck BPM
052413A8 20 278 124     Masterdeck index
0442C0F8 1FC            track_id deck1
0442C0F8 200            track_id deck2
04436DB0 0              bearer
0443F5D0 120 1AC        time deck 1
0443F5D0 128 1AC        time deck 2
```

## Updating
Previously, every Rekordbox update the memory offsets changed. From 7.0.0 -> 7.0.1 the old offsets continued working. 
When the pointers change, I use Cheat Engine, using pointerscans and trying to find the shortest pointer paths.

Easiest method seems to be to find each value, pointerscan, save that, then reopen rekordbox and filter the pointerscans by value. If you can't find any values, try increasing the maximum offset value to something like 32768, offsets = 16. To save performance you can set max level to 5 or 6, paths should not be longer than that.

Updates are welcome, put them in the `offsets` file.

### `master_bpm`
The BPM value of the current master track. Find by loading a track on deck 1 & 2, then search for a float containing the BPM of the deck currently set as Master. Find a value that matches exactly and make sure it doesn't oscillate when you play on that deck.

### `masterdeck_index`
The index of the deck currently set as Master. 0 for deck 1, 1 for deck 2. Not sure if the value I've found is the index of the selected deck, or a boolean dictating if Deck 2 is master. Search for a byte.

This one is usually the trickiest. There are a couple of other values wich correlate but actually change on hover etc., so be careful. The path should not be longer than 4 addresses, so find a bunch of candidates (should be able to reduce to <30) and then pointer scan for each until you get a short one - that should be it.

### `deck1, deck2, bar, beat`
On the waveform view, these are the values "xx.x" showing the current bar and beat. The second to last value in the offset chain is the same per deck, and the last is the same per beat/bar. Thus, if you find Deck1 Bar and Deck2 Beat, you can calculate Deck1 Beat and Deck2 Bar.

### `track_id for deck1 and deck2`
This is the values of track loaded in deck1 and deck2. The values correspond to rekordbox database ids

### `time for deck1 and deck2`
The current timestamp of deck1 and deck2

### `bearer`
This is the token rekordbox uses to communicate with the rekordbox database daemon, reset on every startup. We need this to get the local path of tracks from their id
