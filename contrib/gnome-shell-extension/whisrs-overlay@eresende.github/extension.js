import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const DBUS_INTERFACE = 'org.whisrs.Overlay';
const DBUS_PATH = '/org/whisrs/Overlay';
const STATE_SIGNAL = 'StateChanged';
const LEVEL_SIGNAL = 'LevelChanged';

const OVERLAY_WIDTH = 110;
const OVERLAY_HEIGHT = 40;
const BOTTOM_MARGIN = 24;
const BAR_COUNT = 5;
const BAR_BASELINE = 4;
const BAR_MAX_H = 28;

export default class WhisrsOverlayExtension extends Extension {
    enable() {
        this._actor = new St.Widget({
            style_class: 'whisrs-overlay whisrs-overlay-hidden',
            layout_manager: new Clutter.FixedLayout(),
            reactive: false,
            visible: false,
        });

        this._barsBox = new St.BoxLayout({
            style_class: 'whisrs-overlay-bars',
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._bars = [];
        for (let i = 0; i < BAR_COUNT; i++) {
            const bar = new St.Widget({
                style_class: 'whisrs-overlay-bar',
                y_align: Clutter.ActorAlign.CENTER,
                y_expand: false,
            });
            this._bars.push(bar);
            this._barsBox.add_child(bar);
        }

        this._actor.add_child(this._barsBox);
        Main.uiGroup.add_child(this._actor);

        this._monitorsChangedId = Main.layoutManager.connect(
            'monitors-changed',
            () => this._position()
        );

        this._signalId = Gio.DBus.session.signal_subscribe(
            null, DBUS_INTERFACE, STATE_SIGNAL, DBUS_PATH, null,
            Gio.DBusSignalFlags.NONE,
            (_c, _s, _p, _i, _sig, parameters) => {
                const [state] = parameters.deep_unpack();
                this._setState(state);
            }
        );
        this._levelSignalId = Gio.DBus.session.signal_subscribe(
            null, DBUS_INTERFACE, LEVEL_SIGNAL, DBUS_PATH, null,
            Gio.DBusSignalFlags.NONE,
            (_c, _s, _p, _i, _sig, parameters) => {
                const [level] = parameters.deep_unpack();
                this._setLevel(level);
            }
        );

        this._state = 'idle';
        this._level = 0;
        this._targetLevel = 0;
        this._frame = 0;

        this._position();
    }

    disable() {
        this._stopAnimation();

        if (this._signalId) {
            Gio.DBus.session.signal_unsubscribe(this._signalId);
            this._signalId = 0;
        }
        if (this._levelSignalId) {
            Gio.DBus.session.signal_unsubscribe(this._levelSignalId);
            this._levelSignalId = 0;
        }
        if (this._monitorsChangedId) {
            Main.layoutManager.disconnect(this._monitorsChangedId);
            this._monitorsChangedId = 0;
        }

        this._actor?.destroy();
        this._actor = null;
        this._bars = [];
        this._barsBox = null;
    }

    _setState(state) {
        if (!this._actor)
            return;

        const normalized = String(state).toLowerCase();
        this._actor.remove_style_class_name('whisrs-overlay-recording');
        this._actor.remove_style_class_name('whisrs-overlay-transcribing');
        this._actor.remove_style_class_name('whisrs-overlay-hidden');

        if (normalized === 'recording') {
            this._state = 'recording';
            this._actor.add_style_class_name('whisrs-overlay-recording');
            this._actor.visible = true;
            this._startAnimation();
        } else if (normalized === 'transcribing') {
            this._state = 'transcribing';
            this._actor.add_style_class_name('whisrs-overlay-transcribing');
            this._actor.visible = true;
            this._startAnimation();
        } else {
            this._state = 'idle';
            this._actor.add_style_class_name('whisrs-overlay-hidden');
            // Hide after the snappy CSS fade-out completes.
            GLib.timeout_add(GLib.PRIORITY_DEFAULT, 80, () => {
                if (this._state === 'idle' && this._actor)
                    this._actor.visible = false;
                return GLib.SOURCE_REMOVE;
            });
            this._stopAnimation();
        }
    }

    _position() {
        if (!this._actor)
            return;

        const monitor = Main.layoutManager.primaryMonitor;
        const x = Math.floor(monitor.x + (monitor.width - OVERLAY_WIDTH) / 2);
        const y = Math.floor(monitor.y + monitor.height - OVERLAY_HEIGHT - BOTTOM_MARGIN);
        this._actor.set_position(Math.max(monitor.x, x), Math.max(monitor.y, y));
        this._actor.set_size(OVERLAY_WIDTH, OVERLAY_HEIGHT);

        const cy = Math.floor(OVERLAY_HEIGHT / 2);
        if (this._barsBox) {
            // 5 bars × 4px wide + 4 gaps × 3px = 32px, centered.
            const barBlock = BAR_COUNT * 4 + (BAR_COUNT - 1) * 3;
            const barsX = Math.floor((OVERLAY_WIDTH - barBlock) / 2);
            this._barsBox.set_position(barsX, cy - Math.floor(BAR_MAX_H / 2));
            this._barsBox.set_size(barBlock, BAR_MAX_H);
        }
    }

    _startAnimation() {
        if (this._animationId)
            return;

        this._animationId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 24, () => {
            this._frame++;
            const target = this._targetLevel ?? 0;
            this._level = target > this._level ? target : Math.max(0, this._level * 0.85);
            this._updateBars();
            return GLib.SOURCE_CONTINUE;
        });
        this._updateBars();
    }

    _stopAnimation() {
        if (this._animationId) {
            GLib.Source.remove(this._animationId);
            this._animationId = 0;
        }
    }

    _updateBars() {
        if (!this._bars || this._bars.length === 0)
            return;

        if (this._state === 'recording') {
            const raw = Number.isFinite(this._level) ? this._level : 0;
            const level = Math.max(0, Math.min(1, raw));
            for (let i = 0; i < this._bars.length; i++) {
                const variance = 0.7 + Math.sin(i * 1.7) * 0.3;
                const phase = Math.abs(Math.sin(this._frame / 6.0 + i * 0.9));
                const effective = Math.min(1, Math.max(0, level * variance));
                const dynamic = effective * (0.6 + 0.4 * phase);
                const h = Math.max(BAR_BASELINE, Math.round(BAR_BASELINE + dynamic * (BAR_MAX_H - BAR_BASELINE)));
                this._bars[i].set_height(h);
                this._bars[i].opacity = 255;
            }
        } else if (this._state === 'transcribing') {
            const cycle = BAR_COUNT * 2 - 2;
            const pos = Math.floor(this._frame / 4) % Math.max(1, cycle);
            const active = pos < BAR_COUNT ? pos : cycle - pos;
            for (let i = 0; i < this._bars.length; i++) {
                const dist = Math.abs(i - active);
                const intensity = Math.max(0.18, 1 - dist / 2.5);
                const h = Math.round(BAR_BASELINE + (BAR_MAX_H - BAR_BASELINE) * (0.45 + 0.55 * intensity));
                this._bars[i].set_height(Math.max(BAR_BASELINE, h));
                this._bars[i].opacity = Math.round(255 * intensity);
            }
        }
    }

    _setLevel(level) {
        const numeric = Number(level);
        if (!Number.isFinite(numeric))
            return;

        this._targetLevel = Math.max(0, Math.min(1, numeric));
    }
}
