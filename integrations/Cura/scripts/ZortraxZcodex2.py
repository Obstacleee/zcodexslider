# Cura post-processing script: converts the sliced G-code to a Zortrax .zcodex2 file with ztool.exe.
# Install: copy this file to  <Cura configuration folder>\scripts\  (Cura: Help > Show Configuration Folder),
# restart Cura, then Extensions > Post Processing > Modify G-Code > Add a script > "Zortrax .zcodex2".
# NOT tested inside Cura: check the output folder after slicing and save.
import json
import os
import subprocess
import tempfile

from UM.Application import Application
from UM.Logger import Logger

from ..Script import Script


class ZortraxZcodex2(Script):
    def getSettingDataString(self):
        return json.dumps({
            "name": "Zortrax .zcodex2",
            "key": "ZortraxZcodex2",
            "metadata": {},
            "version": 2,
            "settings": {
                "ztool_path": {
                    "label": "Chemin de ztool.exe",
                    "description": "Chemin complet de ztool.exe",
                    "type": "str",
                    "default_value": "C:\\ztool\\ztool.exe"
                },
                "output_dir": {
                    "label": "Dossier de sortie",
                    "description": "Dossier où écrire le fichier .zcodex2",
                    "type": "str",
                    "default_value": ""
                },
                "material": {
                    "label": "Matériau Zortrax",
                    "description": "Utilisé si le G-code ne donne pas de températures",
                    "type": "enum",
                    "options": {"Z-PLA": "Z-PLA", "Z-PLA Pro": "Z-PLA Pro", "Z-ABS": "Z-ABS", "Z-PETG": "Z-PETG", "Z-HIPS": "Z-HIPS"},
                    "default_value": "Z-PLA"
                }
            }
        })

    def execute(self, data):
        exe = self.getSettingValueByKey("ztool_path")
        out_dir = self.getSettingValueByKey("output_dir")
        material = self.getSettingValueByKey("material")
        if not os.path.isfile(exe):
            Logger.log("e", "ZortraxZcodex2: ztool.exe introuvable : %s", exe)
            return data
        try:
            name = Application.getInstance().getPrintInformation().jobName or "print"
        except Exception:
            name = "print"
        if not out_dir:
            out_dir = os.path.join(os.path.expanduser("~"), "Documents")
        os.makedirs(out_dir, exist_ok=True)
        out_path = os.path.join(out_dir, name + ".zcodex2")

        with tempfile.NamedTemporaryFile("w", suffix=".gcode", delete=False, encoding="utf-8") as f:
            f.write("".join(data))
            tmp = f.name
        try:
            res = subprocess.run([exe, "to-zcodex2", tmp, "-o", out_path, "--material", material, "-q"],
                                 capture_output=True, text=True)
            if res.returncode != 0:
                Logger.log("e", "ZortraxZcodex2: %s", res.stderr.strip())
            else:
                Logger.log("i", "ZortraxZcodex2: écrit %s", out_path)
        finally:
            os.unlink(tmp)
        return data
